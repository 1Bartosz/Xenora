// SPDX-License-Identifier: GPL-3.0-or-later

//! PE32+ Base Relocation Engine for Xenora X15.
//!
//! Applies `IMAGE_REL_BASED_DIR64` relocations to guest virtual memory when an image
//! is loaded at a base address different from its preferred PE `image_base`.

use crate::memory::{GuestMemory, MemoryError};
use crate::pe::ParsedPe;
use std::collections::BTreeMap;
use std::fmt;

/// Type of base relocation entry.
pub const IMAGE_REL_BASED_ABSOLUTE: u8 = 0;
pub const IMAGE_REL_BASED_DIR64: u8 = 10;

/// Errors that can occur during base relocation parsing or application.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RelocationError {
    DirectoryOutOfBounds { rva: u32, size: u32 },
    MalformedBlockHeader { offset: usize },
    BlockSizeOutOfBounds { block_size: u32, max_size: usize },
    UnsupportedRelocationType(u8),
    TargetOutOfBounds { target_va: u64 },
    Memory(MemoryError),
    ArithmeticOverflow,
}

impl fmt::Display for RelocationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DirectoryOutOfBounds { rva, size } => {
                write!(
                    f,
                    "relocation directory (RVA 0x{rva:08x}, size 0x{size:x}) is out of bounds"
                )
            }
            Self::MalformedBlockHeader { offset } => {
                write!(f, "malformed relocation block header at offset {offset}")
            }
            Self::BlockSizeOutOfBounds {
                block_size,
                max_size,
            } => {
                write!(
                    f,
                    "relocation block size 0x{block_size:x} exceeds boundary {max_size}"
                )
            }
            Self::UnsupportedRelocationType(t) => {
                write!(f, "unsupported base relocation type: {t}")
            }
            Self::TargetOutOfBounds { target_va } => {
                write!(
                    f,
                    "relocation target VA 0x{target_va:016x} is outside mapped memory"
                )
            }
            Self::Memory(e) => write!(f, "guest memory fault during relocation: {e}"),
            Self::ArithmeticOverflow => write!(f, "arithmetic overflow in relocation computation"),
        }
    }
}

impl std::error::Error for RelocationError {}

impl From<MemoryError> for RelocationError {
    fn from(e: MemoryError) -> Self {
        Self::Memory(e)
    }
}

/// Statistics and counters from applying base relocations.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RelocationReport {
    pub total_entries: usize,
    pub applied_dir64: usize,
    pub skipped_absolute: usize,
    pub type_counts: BTreeMap<u8, usize>,
    pub delta: i64,
}

/// Applies PE base relocations to `GuestMemory` if the loaded image base differs from preferred.
pub fn apply_relocations(
    pe: &ParsedPe,
    bytes: &[u8],
    loaded_base: u64,
    memory: &mut GuestMemory,
) -> Result<RelocationReport, RelocationError> {
    let mut report = RelocationReport::default();

    let delta = (loaded_base as i128) - (pe.image_base as i128);
    if delta == 0 {
        // Zero delta: image is loaded at preferred base, no DIR64 fixup needed
        return Ok(report);
    }
    if delta > (i64::MAX as i128) || delta < (i64::MIN as i128) {
        return Err(RelocationError::ArithmeticOverflow);
    }
    let delta_i64 = delta as i64;
    report.delta = delta_i64;

    // Locate relocation directory RVA and size from Optional Header data directory #5
    let reloc_dir = find_reloc_directory(bytes, pe)?;
    let Some((reloc_rva, reloc_size)) = reloc_dir else {
        return Ok(report);
    };

    if reloc_size == 0 {
        return Ok(report);
    }

    let reloc_file_offset =
        pe.rva_to_file_offset(reloc_rva)
            .ok_or(RelocationError::DirectoryOutOfBounds {
                rva: reloc_rva,
                size: reloc_size,
            })?;

    if reloc_file_offset + (reloc_size as usize) > bytes.len() {
        return Err(RelocationError::DirectoryOutOfBounds {
            rva: reloc_rva,
            size: reloc_size,
        });
    }

    let mut read_bytes = 0usize;
    let max_read = reloc_size as usize;

    while read_bytes + 8 <= max_read {
        let block_start = reloc_file_offset + read_bytes;
        let page_rva = u32::from_le_bytes(bytes[block_start..block_start + 4].try_into().unwrap());
        let block_size =
            u32::from_le_bytes(bytes[block_start + 4..block_start + 8].try_into().unwrap());

        if block_size < 8 {
            return Err(RelocationError::MalformedBlockHeader {
                offset: block_start,
            });
        }
        if read_bytes + (block_size as usize) > max_read {
            return Err(RelocationError::BlockSizeOutOfBounds {
                block_size,
                max_size: max_read - read_bytes,
            });
        }

        let num_entries = (block_size as usize - 8) / 2;
        for i in 0..num_entries {
            let entry_offset = block_start + 8 + i * 2;
            let entry_val =
                u16::from_le_bytes(bytes[entry_offset..entry_offset + 2].try_into().unwrap());
            let reloc_type = (entry_val >> 12) as u8;
            let offset_in_page = (entry_val & 0x0FFF) as u32;

            *report.type_counts.entry(reloc_type).or_insert(0) += 1;
            report.total_entries += 1;

            match reloc_type {
                IMAGE_REL_BASED_ABSOLUTE => {
                    // Padding entry, ignore
                    report.skipped_absolute += 1;
                }
                IMAGE_REL_BASED_DIR64 => {
                    // Target VA = loaded_base + page_rva + offset_in_page
                    let target_rva = page_rva
                        .checked_add(offset_in_page)
                        .ok_or(RelocationError::ArithmeticOverflow)?;
                    let target_va = loaded_base
                        .checked_add(target_rva as u64)
                        .ok_or(RelocationError::ArithmeticOverflow)?;

                    // Read current 64-bit value at target_va
                    // Use write_u64_bypass / read_u64_bypass if section is read-only
                    let orig_val = memory.read_u64_bypass(target_va).map_err(|e| match e {
                        MemoryError::UnmappedAddress(_) => {
                            RelocationError::TargetOutOfBounds { target_va }
                        }
                        other => RelocationError::Memory(other),
                    })?;

                    // Rebase: new_val = orig_val + delta_i64
                    let new_val = ((orig_val as i128) + (delta_i64 as i128)) as u64;

                    memory
                        .write_u64_bypass(target_va, new_val)
                        .map_err(|e| match e {
                            MemoryError::UnmappedAddress(_) => {
                                RelocationError::TargetOutOfBounds { target_va }
                            }
                            other => RelocationError::Memory(other),
                        })?;

                    report.applied_dir64 += 1;
                }
                other => {
                    return Err(RelocationError::UnsupportedRelocationType(other));
                }
            }
        }

        read_bytes += block_size as usize;
    }

    Ok(report)
}

fn find_reloc_directory(
    bytes: &[u8],
    _pe: &ParsedPe,
) -> Result<Option<(u32, u32)>, RelocationError> {
    if bytes.len() < 0x40 {
        return Ok(None);
    }
    let e_lfanew = u32::from_le_bytes(bytes[0x3C..0x40].try_into().unwrap()) as usize;
    let opt_offset = e_lfanew + 24;
    let coff_offset = e_lfanew + 4;
    if bytes.len() < coff_offset + 18 {
        return Ok(None);
    }
    let size_of_opt = u16::from_le_bytes(
        bytes[coff_offset + 16..coff_offset + 18]
            .try_into()
            .unwrap(),
    ) as usize;

    // Relocation directory is directory index 5 (offset 112 + 5 * 8 = 152 in Optional Header)
    if size_of_opt < 160 || bytes.len() < opt_offset + 160 {
        return Ok(None);
    }

    let reloc_rva = u32::from_le_bytes(
        bytes[opt_offset + 152..opt_offset + 156]
            .try_into()
            .unwrap(),
    );
    let reloc_size = u32::from_le_bytes(
        bytes[opt_offset + 156..opt_offset + 160]
            .try_into()
            .unwrap(),
    );

    if reloc_rva == 0 || reloc_size == 0 {
        Ok(None)
    } else {
        Ok(Some((reloc_rva, reloc_size)))
    }
}
