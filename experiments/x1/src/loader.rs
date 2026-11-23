// SPDX-License-Identifier: GPL-3.0-or-later
//! Generalized PE32+ loader.
//!
//! # What this loader implements
//!
//! - DOS header signature (`MZ`) and PE offset validation.
//! - COFF file header: machine type `IMAGE_FILE_MACHINE_AMD64` (0x8664),
//!   section count, optional-header size, and characteristics.
//! - PE32+ optional header (`Magic` = 0x20B): file alignment, section
//!   alignment, image base, image size, entry-point RVA, and data-directory
//!   count.
//! - Section table: name, virtual size, RVA, raw size, raw offset, and
//!   characteristic flags.
//! - Each section maps its declared virtual size. Bytes beyond the raw
//!   data are zero-filled. Raw bytes beyond the virtual size are file padding.
//! - Section permissions derived from characteristic flags:
//!   - `IMAGE_SCN_CNT_CODE | IMAGE_SCN_MEM_EXECUTE | IMAGE_SCN_MEM_READ`
//!     (0x60000020) → `RX`
//!   - `IMAGE_SCN_CNT_INITIALIZED_DATA | IMAGE_SCN_MEM_READ | IMAGE_SCN_MEM_WRITE`
//!     (0xC0000040) → `RW`
//!   - `IMAGE_SCN_CNT_INITIALIZED_DATA | IMAGE_SCN_MEM_READ` (0x40000040) → `R`
//!   - `IMAGE_SCN_CNT_UNINITIALIZED_DATA | IMAGE_SCN_MEM_READ | IMAGE_SCN_MEM_WRITE`
//!     (0xC0000080) → `RW` (zero-filled)
//! - Checked integer arithmetic and file-offset validation throughout.
//! - Distinct readable, writable, and executable permissions per section.
//! - 1–8 sections.
//!
//! # What this loader rejects
//!
//! - Machine types other than AMD64 (0x8664).
//! - 32-bit PE (Magic = 0x10B).
//! - Image bases that are not a multiple of 0x10000 (64 KB), or above
//!   `0x7FFF_FFFF_0000` (conservative user-space limit).
//! - Sections with zero virtual size.
//! - Overlapping sections (by RVA or by raw file offset).
//! - Sections whose raw data extends past the end of the file.
//! - Sections with unsupported flag combinations.
//! - Entry points outside the mapped image.
//! - Entry points in non-executable sections.
//! - Entry points that fall in zero-initialised virtual tails.
//! - Nonzero data directories, including imports, base relocations, and TLS.
//! - Truncated or malformed headers.
//! - Uninitialized-data sections with nonzero raw offsets or sizes.
//!
//! # What is not implemented
//!
//! - Import resolution (IAT/INT).
//! - Base relocations.
//! - Debug directories, bound imports, TLS, resources, exceptions.
//! - Delay-load imports.
//! - Thread-local storage.
//! - Overlay data beyond the last section.

use crate::mem::{GuestMemory, Perm};

/// Section count limits.
const MIN_SECTIONS: u16 = 1;
const MAX_SECTIONS: u16 = 8;

/// Optional-header size for PE32+ (IMAGE_OPTIONAL_HEADER64).
const OPTIONAL_HEADER_SIZE: u16 = 240;

/// Number of data directories included by default in PE32+.
const NUM_DATA_DIRS: u32 = 16;

/// Section flags we recognise and their corresponding permissions.
const SCN_RX: u32 = 0x6000_0020; // code + exec + read
const SCN_RW: u32 = 0xC000_0040; // init data + read + write
const SCN_R: u32 = 0x4000_0040; // init data + read
const SCN_RW_BSS: u32 = 0xC000_0080; // uninit data + read + write

/// Conservative upper bound for image base (below 128 TB user space).
const IMAGE_BASE_MAX: u64 = 0x0000_7FFF_FFFF_0000;

/// Maximum image size we are willing to load (64 MB).
const IMAGE_SIZE_MAX: u32 = 64 * 1024 * 1024;

// ── error type ───────────────────────────────────────────────────────────────

/// Reason a load attempt failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LoadError {
    /// A required field was outside the file bounds.
    Bounds,
    /// A header field had an unsupported or inconsistent value.
    Header,
    /// A data directory was present that this loader cannot handle.
    Directory,
    /// A section header or its raw data was invalid.
    Section,
    /// The entry-point RVA was out of range or landed in non-executable memory.
    Entry,
    /// Two sections overlapped in RVA space or in raw file space.
    Overlap,
}

// ── byte-level helpers ───────────────────────────────────────────────────────

fn read_u16(buf: &[u8], off: usize) -> Result<u16, LoadError> {
    buf.get(off..off.checked_add(2).ok_or(LoadError::Bounds)?)
        .and_then(|s| s.try_into().ok())
        .map(u16::from_le_bytes)
        .ok_or(LoadError::Bounds)
}

fn read_u32(buf: &[u8], off: usize) -> Result<u32, LoadError> {
    buf.get(off..off.checked_add(4).ok_or(LoadError::Bounds)?)
        .and_then(|s| s.try_into().ok())
        .map(u32::from_le_bytes)
        .ok_or(LoadError::Bounds)
}

fn read_u64(buf: &[u8], off: usize) -> Result<u64, LoadError> {
    buf.get(off..off.checked_add(8).ok_or(LoadError::Bounds)?)
        .and_then(|s| s.try_into().ok())
        .map(u64::from_le_bytes)
        .ok_or(LoadError::Bounds)
}

// ── parsed PE metadata ───────────────────────────────────────────────────────

/// Parsed section descriptor before memory mapping.
#[derive(Debug)]
struct SectionDesc {
    /// Virtual address (image base + RVA).
    va: u64,
    /// Number of bytes to map in the virtual address space (virtual size).
    virtual_size: usize,
    /// Bytes to copy from the file (min of raw size and virtual size).
    raw_size: usize,
    /// Offset within the file image slice from which to copy.
    raw_off: usize,
    raw_size_file: usize,
    /// Permission to apply to this region.
    perm: Perm,
    /// True when this section is entirely zero-initialised (BSS).
    bss: bool,
}

// ── public API ───────────────────────────────────────────────────────────────

/// A successfully loaded PE32+ image mapped into `GuestMemory`.
#[derive(Debug)]
pub(crate) struct LoadedImage {
    /// Guest virtual address of the entrypoint.
    pub(crate) entry: u64,
    /// Guest virtual address of the image base.
    pub(crate) image_base: u64,
    /// Guest memory holding all mapped sections.
    pub(crate) mem: GuestMemory,
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Directory {
    pub(crate) rva: u32,
    pub(crate) size: u32,
}

pub(crate) struct ImportDirectories {
    pub(crate) descriptors: Directory,
    pub(crate) address_table: Directory,
}

pub(crate) fn load(image: &[u8]) -> Result<LoadedImage, LoadError> {
    parse(image, false).map(|(image, _)| image)
}

pub(crate) fn load_imports(image: &[u8]) -> Result<(LoadedImage, ImportDirectories), LoadError> {
    let (image, directories) = parse(image, true)?;
    Ok((image, directories.ok_or(LoadError::Directory)?))
}

fn parse(
    image: &[u8],
    allow_imports: bool,
) -> Result<(LoadedImage, Option<ImportDirectories>), LoadError> {
    // ── DOS header ──────────────────────────────────────────────────────────
    if read_u16(image, 0)? != 0x5A4D {
        // "MZ"
        return Err(LoadError::Header);
    }
    let pe_off = read_u32(image, 0x3C)? as usize;
    // The PE header must start on an 8-byte boundary, be at least 0x40,
    // and be entirely within the file.
    if pe_off < 0x40 || !pe_off.is_multiple_of(8) {
        return Err(LoadError::Header);
    }

    // ── COFF header (at pe_off) ─────────────────────────────────────────────
    // Signature: "PE\0\0"
    if image
        .get(pe_off..pe_off.checked_add(4).ok_or(LoadError::Bounds)?)
        .ok_or(LoadError::Bounds)?
        != b"PE\0\0"
    {
        return Err(LoadError::Header);
    }
    let coff = pe_off + 4;
    let machine = read_u16(image, coff)?;
    if machine != 0x8664 {
        return Err(LoadError::Header); // must be AMD64
    }
    let section_count = read_u16(image, coff + 2)?;
    if !(MIN_SECTIONS..=MAX_SECTIONS).contains(&section_count) {
        return Err(LoadError::Header);
    }
    let opt_size = read_u16(image, coff + 16)?;
    if opt_size != OPTIONAL_HEADER_SIZE {
        return Err(LoadError::Header);
    }
    let characteristics = read_u16(image, coff + 18)?;
    // Must be executable (IMAGE_FILE_EXECUTABLE_IMAGE = 0x0002).
    if characteristics & 0x0002 == 0 || characteristics & 0x2000 != 0 {
        return Err(LoadError::Header);
    }

    // ── Optional header (PE32+) ─────────────────────────────────────────────
    let opt = coff + 20;
    let magic = read_u16(image, opt)?;
    if magic != 0x020B {
        return Err(LoadError::Header); // must be PE32+
    }
    let file_alignment = read_u32(image, opt + 36)?;
    // File alignment must be a power of two in [512, 65536].
    if !(512..=65536).contains(&file_alignment) || !file_alignment.is_power_of_two() {
        return Err(LoadError::Header);
    }
    let section_alignment = read_u32(image, opt + 32)?;
    // Section alignment must be >= file_alignment and a power of two.
    if section_alignment < file_alignment
        || !section_alignment.is_power_of_two()
        || (section_alignment < 0x1000 && section_alignment != file_alignment)
    {
        return Err(LoadError::Header);
    }
    let image_base = read_u64(image, opt + 24)?;
    if image_base == 0 || !image_base.is_multiple_of(0x10000) || image_base > IMAGE_BASE_MAX {
        return Err(LoadError::Header);
    }
    let entry_rva = read_u32(image, opt + 16)?;
    let image_size = read_u32(image, opt + 56)?;
    if image_size == 0
        || image_size > IMAGE_SIZE_MAX
        || !image_size.is_multiple_of(section_alignment)
        || image_base + u64::from(image_size) > 0x0000_8000_0000_0000
    {
        return Err(LoadError::Header);
    }
    let headers_size = read_u32(image, opt + 60)?;
    let num_dirs = read_u32(image, opt + 108)?;
    if num_dirs > NUM_DATA_DIRS {
        return Err(LoadError::Header);
    }

    let mut descriptors = Directory::default();
    let mut address_table = Directory::default();
    for dir_idx in 0..num_dirs as usize {
        let dir_off = opt + 112 + dir_idx * 8;
        let rva = read_u32(image, dir_off)?;
        let size = read_u32(image, dir_off + 4)?;
        match (allow_imports, dir_idx, rva, size) {
            (_, _, 0, 0) => {}
            (true, 1, 1.., 1..) => descriptors = Directory { rva, size },
            (true, 12, 1.., 1..) => address_table = Directory { rva, size },
            _ => return Err(LoadError::Directory),
        }
    }
    let directories = if allow_imports {
        for directory in [descriptors, address_table] {
            if directory.rva == 0
                || directory
                    .rva
                    .checked_add(directory.size)
                    .is_none_or(|end| end > image_size)
            {
                return Err(LoadError::Directory);
            }
        }
        Some(ImportDirectories {
            descriptors,
            address_table,
        })
    } else {
        None
    };

    // ── Section table ───────────────────────────────────────────────────────
    let table_off = opt + opt_size as usize;
    let table_end = table_off
        .checked_add(section_count as usize * 40)
        .ok_or(LoadError::Bounds)?;
    if table_end > headers_size as usize || headers_size as usize > image.len() {
        return Err(LoadError::Bounds);
    }

    if headers_size >= image_size || !headers_size.is_multiple_of(file_alignment) {
        return Err(LoadError::Header);
    }

    let mut sections: Vec<SectionDesc> = Vec::with_capacity(section_count as usize);

    for i in 0..section_count as usize {
        let hdr = &image[table_off + i * 40..table_off + (i + 1) * 40];

        // Virtual size (must be non-zero).
        let virt_sz = read_u32(hdr, 8)? as usize;
        if virt_sz == 0 {
            return Err(LoadError::Section);
        }
        let rva = read_u32(hdr, 12)?;
        if rva < headers_size || !rva.is_multiple_of(section_alignment) {
            return Err(LoadError::Section);
        }
        let raw_sz_file = read_u32(hdr, 16)? as usize;
        let raw_off = read_u32(hdr, 20)? as usize;
        let flags = read_u32(hdr, 36)?;

        // Translate flags to permissions.
        let (perm, bss) = match flags {
            SCN_RX => (Perm::RX, false),
            SCN_RW => (Perm::RW, false),
            SCN_R => (Perm::READ, false),
            SCN_RW_BSS => (Perm::RW, true),
            _ => return Err(LoadError::Section),
        };

        // VA = image_base + rva.
        let va = image_base
            .checked_add(u64::from(rva))
            .ok_or(LoadError::Section)?;

        let aligned_size = virt_sz.next_multiple_of(section_alignment as usize);
        if rva as usize + aligned_size > image_size as usize {
            return Err(LoadError::Section);
        }

        // For non-BSS sections, the raw data must be within the file.
        let raw_size = if bss {
            // BSS sections have no raw bytes.
            if raw_off != 0 || raw_sz_file != 0 {
                return Err(LoadError::Section);
            }
            0
        } else {
            if raw_sz_file == 0
                || raw_off < headers_size as usize
                || !raw_off.is_multiple_of(file_alignment as usize)
                || !raw_sz_file.is_multiple_of(file_alignment as usize)
                || (section_alignment < 0x1000 && raw_off != rva as usize)
            {
                return Err(LoadError::Section);
            }
            let raw_end = raw_off.checked_add(raw_sz_file).ok_or(LoadError::Section)?;
            if raw_end > image.len() {
                return Err(LoadError::Section);
            }
            // Bytes to copy = min(raw_sz_file, virt_sz).
            raw_sz_file.min(virt_sz)
        };

        sections.push(SectionDesc {
            va,
            virtual_size: virt_sz,
            raw_size,
            raw_off,
            raw_size_file: raw_sz_file,
            perm,
            bss,
        });
    }

    // ── Overlap checks ──────────────────────────────────────────────────────
    // Sort by VA and check for overlap in virtual space.
    {
        let mut vas: Vec<(u64, usize, usize)> = sections
            .iter()
            .enumerate()
            .map(|(i, s)| (s.va, s.virtual_size, i))
            .collect();
        vas.sort_by_key(|&(va, _, _)| va);
        for w in vas.windows(2) {
            let (va_a, sz_a, _) = w[0];
            let (va_b, _, _) = w[1];
            let end_a = va_a.checked_add(sz_a as u64).ok_or(LoadError::Overlap)?;
            if va_b < end_a {
                return Err(LoadError::Overlap);
            }
        }
    }
    // Check for overlap in raw file space (non-BSS sections only).
    {
        let mut raws: Vec<(usize, usize)> = sections
            .iter()
            .filter(|s| !s.bss && s.raw_size > 0)
            .map(|s| (s.raw_off, s.raw_size_file))
            .collect();
        raws.sort_by_key(|&(off, _)| off);
        for w in raws.windows(2) {
            let (off_a, sz_a) = w[0];
            let (off_b, _) = w[1];
            if off_b < off_a + sz_a {
                return Err(LoadError::Overlap);
            }
        }
    }

    // ── Validate entry point ────────────────────────────────────────────────
    if entry_rva == 0 {
        return Err(LoadError::Entry);
    }
    let entry_va = image_base
        .checked_add(u64::from(entry_rva))
        .ok_or(LoadError::Entry)?;
    // Entry must land in a section with EXEC permission, and within the
    // virtual size (not in the zero-filled tail).
    let entry_ok = sections
        .iter()
        .any(|s| s.perm.has(Perm::EXEC) && entry_va >= s.va && entry_va < s.va + s.raw_size as u64);
    if !entry_ok {
        return Err(LoadError::Entry);
    }

    // ── Map sections into guest memory ──────────────────────────────────────
    let mut mem = GuestMemory::new();
    for s in &sections {
        if !mem.map(s.va, s.virtual_size, s.perm, 0) {
            return Err(LoadError::Section);
        }
        if !s.bss && s.raw_size > 0 {
            mem.initialize(s.va, &image[s.raw_off..s.raw_off + s.raw_size])
                .map_err(|_| LoadError::Section)?;
        }
        // The virtual tail [va+raw_size .. va+virtual_size] stays zero (from
        // the fill value of 0 above).
    }

    Ok((
        LoadedImage {
            entry: entry_va,
            image_base,
            mem,
        },
        directories,
    ))
}

// ── tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::mem::MemFault;

    // ── builder ──────────────────────────────────────────────────────────────

    /// Minimal in-memory PE32+ builder for tests.
    /// Sections: list of (name8, payload, flags).
    pub(crate) fn build_pe(image_base: u64, sections: &[(&[u8; 8], &[u8], u32)]) -> Vec<u8> {
        let n = sections.len() as u16;
        let file_align: u32 = 0x200;
        let sect_align: u32 = 0x1000;
        // Total file size: header page + one page per section.
        let total = 0x200 * (1 + sections.len());
        let mut image = vec![0u8; total];

        // DOS header.
        image[0..2].copy_from_slice(b"MZ");
        image[0x3C..0x40].copy_from_slice(&0x80u32.to_le_bytes());

        // COFF header at 0x80.
        let pe = 0x80usize;
        image[pe..pe + 4].copy_from_slice(b"PE\0\0");
        image[pe + 4..pe + 6].copy_from_slice(&0x8664u16.to_le_bytes()); // machine
        image[pe + 6..pe + 8].copy_from_slice(&n.to_le_bytes()); // section count
        // timestamp = 0, symbol table = 0, symbol count = 0.
        image[pe + 20..pe + 22].copy_from_slice(&OPTIONAL_HEADER_SIZE.to_le_bytes()); // opt size
        image[pe + 22..pe + 24].copy_from_slice(&0x0022u16.to_le_bytes()); // characteristics

        // Optional header at pe+24 (= 0x98).
        let opt = pe + 24;
        image[opt..opt + 2].copy_from_slice(&0x020Bu16.to_le_bytes()); // magic PE32+
        // MajorLinkerVersion, MinorLinkerVersion = 0.
        // SizeOfCode.
        let code_size: u32 = sections
            .iter()
            .filter(|(_, _, f)| f & 0x20 != 0)
            .map(|(_, p, _)| p.len() as u32)
            .sum();
        image[opt + 4..opt + 8].copy_from_slice(&code_size.to_le_bytes());
        // SizeOfInitializedData — sum of non-code sections.
        let idata_size: u32 = sections
            .iter()
            .filter(|(_, _, f)| f & 0x20 == 0)
            .map(|(_, p, _)| p.len() as u32)
            .sum();
        image[opt + 8..opt + 12].copy_from_slice(&idata_size.to_le_bytes());
        // SizeOfUninitializedData = 0.
        // AddressOfEntryPoint = first section RVA = 0x1000.
        image[opt + 16..opt + 20].copy_from_slice(&sect_align.to_le_bytes());
        // BaseOfCode = first section RVA.
        image[opt + 20..opt + 24].copy_from_slice(&sect_align.to_le_bytes());
        // ImageBase.
        image[opt + 24..opt + 32].copy_from_slice(&image_base.to_le_bytes());
        // SectionAlignment.
        image[opt + 32..opt + 36].copy_from_slice(&sect_align.to_le_bytes());
        // FileAlignment.
        image[opt + 36..opt + 40].copy_from_slice(&file_align.to_le_bytes());
        // OS/image/subsystem versions — 0.
        // Win32VersionValue = 0.
        // SizeOfImage = (1 + n) * sect_align.
        let image_size: u32 = sect_align * (1 + n as u32);
        image[opt + 56..opt + 60].copy_from_slice(&image_size.to_le_bytes());
        // SizeOfHeaders = file_align.
        image[opt + 60..opt + 64].copy_from_slice(&file_align.to_le_bytes());
        // Checksum = 0.
        // Subsystem = 3 (console).
        image[opt + 68..opt + 70].copy_from_slice(&3u16.to_le_bytes());
        // DllCharacteristics = 0.
        // Stack/heap reserve/commit — 0.
        // NumberOfRvaAndSizes = 16.
        image[opt + 108..opt + 112].copy_from_slice(&16u32.to_le_bytes());
        // All 16 data directories = 0 (no imports, no relocations).

        // Section table.
        let table = pe + 24 + OPTIONAL_HEADER_SIZE as usize; // = pe + 264
        for (i, (name, payload, flags)) in sections.iter().enumerate() {
            let raw_off: u32 = file_align * (1 + i as u32);
            let rva: u32 = sect_align * (1 + i as u32);
            let hdr_off = table + i * 40;
            image[hdr_off..hdr_off + 8].copy_from_slice(*name);
            image[hdr_off + 8..hdr_off + 12].copy_from_slice(&(payload.len() as u32).to_le_bytes()); // VirtualSize
            image[hdr_off + 12..hdr_off + 16].copy_from_slice(&rva.to_le_bytes());
            image[hdr_off + 16..hdr_off + 20].copy_from_slice(&file_align.to_le_bytes()); // SizeOfRawData
            image[hdr_off + 20..hdr_off + 24].copy_from_slice(&raw_off.to_le_bytes());
            // PointerToRelocations, PointerToLinenumbers, NumberOfRelocations,
            // NumberOfLinenumbers = 0.
            image[hdr_off + 36..hdr_off + 40].copy_from_slice(&flags.to_le_bytes());

            // Copy payload into raw page.
            let raw = raw_off as usize;
            image[raw..raw + payload.len()].copy_from_slice(payload);
        }

        image
    }

    /// Minimal code that can serve as a valid `.text` payload: a single
    /// `NOP` byte (0x90). The entry point lands here and the CPU will fault
    /// on `Unsupported` immediately — that is fine for loader tests.
    const NOP: &[u8] = &[0x90];

    #[test]
    fn loads_single_code_section() {
        let image = build_pe(0x1_4000_0000, &[(b".text\0\0\0", NOP, SCN_RX)]);
        let loaded = load(&image).unwrap();
        assert_eq!(loaded.image_base, 0x1_4000_0000);
        assert_eq!(loaded.entry, 0x1_4000_1000);
    }

    #[test]
    fn loads_code_and_data_sections() {
        let data = [0xA5u8; 8];
        let image = build_pe(
            0x1_4000_0000,
            &[
                (b".text\0\0\0", NOP, SCN_RX),
                (b".data\0\0\0", &data, SCN_RW),
            ],
        );
        let loaded = load(&image).unwrap();
        assert_eq!(loaded.entry, 0x1_4000_1000);
        // Data section must be readable.
        assert_eq!(loaded.mem.load_u8(0x1_4000_2000).unwrap(), 0xA5);
    }

    #[test]
    fn loads_three_sections_with_read_only() {
        let image = build_pe(
            0x1_4000_0000,
            &[
                (b".text\0\0\0", NOP, SCN_RX),
                (b".data\0\0\0", NOP, SCN_RW),
                (b".rdata\0\0", NOP, SCN_R),
            ],
        );
        let mut loaded = load(&image).unwrap();
        // .rdata is not writable.
        assert_eq!(
            loaded.mem.store_u8(0x1_4000_3000, 0).unwrap_err(),
            MemFault::NotWritable
        );
        // .rdata is not executable.
        assert_eq!(
            loaded.mem.fetch(0x1_4000_3000, 1).unwrap_err(),
            MemFault::NotExecutable
        );
    }

    #[test]
    fn rejects_bad_dos_magic() {
        let mut image = build_pe(0x1_4000_0000, &[(b".text\0\0\0", NOP, SCN_RX)]);
        image[0] = 0;
        assert_eq!(load(&image).unwrap_err(), LoadError::Header);
    }

    #[test]
    fn rejects_bad_pe_signature() {
        let mut image = build_pe(0x1_4000_0000, &[(b".text\0\0\0", NOP, SCN_RX)]);
        image[0x81] = 0; // corrupt 'E'
        assert_eq!(load(&image).unwrap_err(), LoadError::Header);
    }

    #[test]
    fn rejects_non_amd64_machine() {
        let mut image = build_pe(0x1_4000_0000, &[(b".text\0\0\0", NOP, SCN_RX)]);
        // machine at pe+4 = 0x84; change to 0x14C (i386).
        image[0x84..0x86].copy_from_slice(&0x014Cu16.to_le_bytes());
        assert_eq!(load(&image).unwrap_err(), LoadError::Header);
    }

    #[test]
    fn rejects_32bit_magic() {
        let mut image = build_pe(0x1_4000_0000, &[(b".text\0\0\0", NOP, SCN_RX)]);
        // Magic is at opt = pe+24 = 0xA8; change to 0x10B.
        image[0x98..0x9A].copy_from_slice(&0x010Bu16.to_le_bytes());
        assert_eq!(load(&image).unwrap_err(), LoadError::Header);
    }

    #[test]
    fn rejects_nonzero_import_directory() {
        let mut image = build_pe(0x1_4000_0000, &[(b".text\0\0\0", NOP, SCN_RX)]);
        // Data directory 1 (import) is at opt+96 + 1*8 = opt+104.
        // opt = 0x98 in a header at 0x80; opt = 0x80+24 = 0x98.
        // dir_base = opt + 112 = 0x108; dir 1 = 0x108 + 8 = 0x110.
        image[0x110..0x114].copy_from_slice(&1u32.to_le_bytes()); // rva
        assert_eq!(load(&image).unwrap_err(), LoadError::Directory);
    }

    #[test]
    fn rejects_nonzero_base_relocation_directory() {
        let mut image = build_pe(0x1_4000_0000, &[(b".text\0\0\0", NOP, SCN_RX)]);
        image[0x130] = 1;
        assert_eq!(load(&image).unwrap_err(), LoadError::Directory);
    }

    #[test]
    fn rejects_tls_directory_before_guest_startup() {
        let mut image = build_pe(0x1_4000_0000, &[(b".text\0\0\0", NOP, SCN_RX)]);
        image[0x150] = 1;
        assert_eq!(load(&image).unwrap_err(), LoadError::Directory);
    }

    #[test]
    fn rejects_overlapping_virtual_or_raw_sections() {
        let image = build_pe(
            0x1_4000_0000,
            &[
                (b".text\0\0\0", NOP, SCN_RX),
                (b".data\0\0\0", &[0], SCN_RW),
            ],
        );
        for (offset, value) in [(0x1BC, 0x1000u32), (0x1C4, 0x200u32)] {
            let mut overlapping = image.clone();
            overlapping[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
            assert_eq!(load(&overlapping).unwrap_err(), LoadError::Overlap);
        }
    }

    #[test]
    fn initialized_section_zero_fills_its_virtual_tail() {
        let mut image = build_pe(
            0x1_4000_0000,
            &[
                (b".text\0\0\0", NOP, SCN_RX),
                (b".data\0\0\0", &[0xA5], SCN_RW),
            ],
        );
        image[0x1B8..0x1BC].copy_from_slice(&0x210u32.to_le_bytes());
        let loaded = load(&image).unwrap();
        assert_eq!(loaded.mem.load_u8(0x1_4000_2000), Ok(0xA5));
        assert_eq!(loaded.mem.read_bytes(0x1_4000_2200, 16).unwrap(), [0; 16]);
        assert_eq!(loaded.mem.load_u8(0x1_4000_2210), Err(MemFault::Unmapped));
    }

    #[test]
    fn rejects_zero_virtual_size() {
        let mut image = build_pe(0x1_4000_0000, &[(b".text\0\0\0", NOP, SCN_RX)]);
        // VirtualSize at section 0 offset 8 within section header.
        // table = pe + 264 = 0x80 + 264 = 0x188.
        image[0x190..0x194].copy_from_slice(&0u32.to_le_bytes());
        assert_eq!(load(&image).unwrap_err(), LoadError::Section);
    }

    #[test]
    fn rejects_unsupported_section_flags() {
        let mut image = build_pe(0x1_4000_0000, &[(b".text\0\0\0", NOP, SCN_RX)]);
        // Replace .text flags with something we don't support.
        image[0x1AC..0x1B0].copy_from_slice(&0x2000_0020u32.to_le_bytes());
        assert_eq!(load(&image).unwrap_err(), LoadError::Section);
    }

    #[test]
    fn rejects_entry_in_data_section() {
        // Build with .data first so entry_rva (0x1000) hits .data, not .text.
        // We achieve this by swapping order: data at RVA 0x1000, text at 0x2000.
        // The easiest way is a custom image where entry_rva points to .data.
        let mut image = build_pe(
            0x1_4000_0000,
            &[
                (b".text\0\0\0", NOP, SCN_RX),
                (b".data\0\0\0", &[0u8; 8], SCN_RW),
            ],
        );
        // Patch entry_rva (opt+16) to point into .data (RVA 0x2000).
        // opt = 0x98; opt+16 = 0xB4.
        image[0xA8..0xAC].copy_from_slice(&0x2000u32.to_le_bytes());
        assert_eq!(load(&image).unwrap_err(), LoadError::Entry);
    }

    #[test]
    fn zero_entry_rva_rejected() {
        let mut image = build_pe(0x1_4000_0000, &[(b".text\0\0\0", NOP, SCN_RX)]);
        // opt+16 = 0xB4.
        image[0xA8..0xAC].copy_from_slice(&0u32.to_le_bytes());
        assert_eq!(load(&image).unwrap_err(), LoadError::Entry);
    }

    #[test]
    fn truncated_image_returns_bounds() {
        let image = build_pe(0x1_4000_0000, &[(b".text\0\0\0", NOP, SCN_RX)]);
        for len in 0..image.len() {
            let r = load(&image[..len]);
            // Should always be an error for any truncation.
            assert!(r.is_err(), "expected error for len={len}, got Ok");
        }
    }

    #[test]
    fn accepts_unaligned_pe_offset_rejected() {
        // pe_off = 0x81 (not 8-byte aligned) → Header error.
        let mut image = build_pe(0x1_4000_0000, &[(b".text\0\0\0", NOP, SCN_RX)]);
        image[0x3C..0x40].copy_from_slice(&0x81u32.to_le_bytes());
        assert_eq!(load(&image).unwrap_err(), LoadError::Header);
    }
}
