// SPDX-License-Identifier: GPL-3.0-or-later

//! Guest virtual memory model for Xenora's x86-64 execution bootstrap.
//!
//! Provides isolated, page-aligned guest virtual memory with permission checking
//! (Read, Write, Execute) and bounds enforcement without host OS mapping tricks.

use std::fmt;

pub(crate) const ALLOCATION_GRANULARITY: u64 = 65_536;

/// Permission flags for a mapped memory region.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MemoryPermissions {
    pub read: bool,
    pub write: bool,
    pub execute: bool,
}

impl MemoryPermissions {
    pub const NO_ACCESS: Self = Self {
        read: false,
        write: false,
        execute: false,
    };
    pub const READ_ONLY: Self = Self {
        read: true,
        write: false,
        execute: false,
    };
    pub const READ_WRITE: Self = Self {
        read: true,
        write: true,
        execute: false,
    };
    pub const READ_EXECUTE: Self = Self {
        read: true,
        write: false,
        execute: true,
    };
    pub const READ_WRITE_EXECUTE: Self = Self {
        read: true,
        write: true,
        execute: true,
    };
}

/// Errors occurring during guest memory operations.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MemoryError {
    UnmappedAddress(u64),
    PermissionViolation {
        address: u64,
        required: &'static str,
    },
    OverlappingRegion {
        base: u64,
        size: u64,
    },
    Overflow,
    AllocationFailed,
}

impl fmt::Display for MemoryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnmappedAddress(addr) => write!(f, "unmapped guest address 0x{addr:016x}"),
            Self::PermissionViolation { address, required } => {
                write!(
                    f,
                    "permission violation at guest address 0x{address:016x}: requires {required}"
                )
            }
            Self::OverlappingRegion { base, size } => {
                write!(
                    f,
                    "overlapping memory region: base 0x{base:016x}, size 0x{size:x}"
                )
            }
            Self::Overflow => write!(f, "guest memory address calculation overflow"),
            Self::AllocationFailed => write!(f, "guest memory allocation exceeds allowed limit"),
        }
    }
}

impl std::error::Error for MemoryError {}

#[derive(Clone, Copy, PartialEq, Eq)]
enum AllocationKind {
    ImageOrStatic,
    Virtual,
    Heap,
}

#[derive(Clone)]
struct MappedRegion {
    base: u64,
    data: Vec<u8>,
    permissions: MemoryPermissions,
    allocation: AllocationKind,
}

impl MappedRegion {
    fn contains(&self, address: u64, size: usize) -> bool {
        let size_u64 = size as u64;
        let Some(end) = address.checked_add(size_u64) else {
            return false;
        };
        let region_end = self.base + self.data.len() as u64;
        address >= self.base && end <= region_end
    }
}

/// Guest virtual memory address space.
#[derive(Default, Clone)]
pub struct GuestMemory {
    regions: Vec<MappedRegion>,
}

impl GuestMemory {
    /// Create a new empty guest address space.
    pub fn new() -> Self {
        Self {
            regions: Vec::new(),
        }
    }

    /// Map a new region into guest virtual memory.
    pub fn map_region(
        &mut self,
        base: u64,
        size: usize,
        permissions: MemoryPermissions,
    ) -> Result<(), MemoryError> {
        self.map_region_with_data(base, size, permissions, &[])
    }

    /// Map a new region and initialize it with initial data, bypassing runtime write permission checks.
    pub fn map_region_with_data(
        &mut self,
        base: u64,
        size: usize,
        permissions: MemoryPermissions,
        initial_data: &[u8],
    ) -> Result<(), MemoryError> {
        self.insert_region(
            base,
            size,
            permissions,
            initial_data,
            AllocationKind::ImageOrStatic,
        )
    }

    fn insert_region(
        &mut self,
        base: u64,
        size: usize,
        permissions: MemoryPermissions,
        initial_data: &[u8],
        allocation: AllocationKind,
    ) -> Result<(), MemoryError> {
        let size_u64 = size as u64;
        let end = base.checked_add(size_u64).ok_or(MemoryError::Overflow)?;
        for r in &self.regions {
            let r_end = r.base + r.data.len() as u64;
            if base < r_end && r.base < end {
                return Err(MemoryError::OverlappingRegion {
                    base,
                    size: size_u64,
                });
            }
        }

        let mut data = Vec::new();
        data.try_reserve_exact(size)
            .map_err(|_| MemoryError::AllocationFailed)?;
        data.resize(size, 0);
        let copy_len = size.min(initial_data.len());
        data[..copy_len].copy_from_slice(&initial_data[..copy_len]);

        self.regions.push(MappedRegion {
            base,
            data,
            permissions,
            allocation,
        });
        self.regions.sort_by_key(|r| r.base);
        Ok(())
    }

    pub(crate) fn map_virtual_allocation(
        &mut self,
        requested_base: u64,
        size: usize,
        permissions: MemoryPermissions,
    ) -> Result<u64, MemoryError> {
        self.map_dynamic_allocation(requested_base, size, permissions, AllocationKind::Virtual)
    }

    pub(crate) fn map_heap_allocation(&mut self, size: usize) -> Result<u64, MemoryError> {
        self.map_dynamic_allocation(0, size, MemoryPermissions::READ_WRITE, AllocationKind::Heap)
    }

    fn map_dynamic_allocation(
        &mut self,
        requested_base: u64,
        size: usize,
        permissions: MemoryPermissions,
        allocation: AllocationKind,
    ) -> Result<u64, MemoryError> {
        // Bound retained host storage independently for each allocation kind.
        const MAX_VIRTUAL_BYTES: usize = 64 * 1024 * 1024;
        let allocated: usize = self
            .regions
            .iter()
            .filter(|region| region.allocation == allocation)
            .map(|region| region.data.len())
            .sum();
        if size > MAX_VIRTUAL_BYTES - allocated
            || (allocation == AllocationKind::Heap
                && self
                    .regions
                    .iter()
                    .filter(|r| r.allocation == allocation)
                    .count()
                    >= 4096)
        {
            return Err(MemoryError::AllocationFailed);
        }

        let mut base = requested_base;
        if base == 0 {
            base = 0x0000_0003_0000_0000;
            for region in &self.regions {
                let end = base.checked_add(size as u64).ok_or(MemoryError::Overflow)?;
                if end <= region.base {
                    break;
                }
                let region_end = region.base + region.data.len() as u64;
                if base < region_end {
                    base = region_end
                        .checked_next_multiple_of(ALLOCATION_GRANULARITY)
                        .ok_or(MemoryError::Overflow)?;
                }
            }
        }
        self.insert_region(base, size, permissions, &[], allocation)?;
        Ok(base)
    }

    pub(crate) fn release_virtual_allocation(&mut self, base: u64) -> bool {
        self.release_dynamic_allocation(base, AllocationKind::Virtual)
    }

    pub(crate) fn release_heap_allocation(&mut self, base: u64) -> bool {
        self.release_dynamic_allocation(base, AllocationKind::Heap)
    }

    pub(crate) fn heap_allocation_size(&self, base: u64) -> Option<usize> {
        self.regions
            .iter()
            .find(|region| region.base == base && region.allocation == AllocationKind::Heap)
            .map(|region| region.data.len())
    }

    fn release_dynamic_allocation(&mut self, base: u64, allocation: AllocationKind) -> bool {
        let Some(index) = self
            .regions
            .iter()
            .position(|region| region.base == base && region.allocation == allocation)
        else {
            return false;
        };
        self.regions.remove(index);
        true
    }

    /// Write raw bytes into guest memory at the specified address, checking permissions.
    pub fn write_bytes(&mut self, address: u64, src: &[u8]) -> Result<(), MemoryError> {
        for region in &mut self.regions {
            if region.contains(address, src.len()) {
                if !region.permissions.write {
                    return Err(MemoryError::PermissionViolation {
                        address,
                        required: "write",
                    });
                }
                let offset = (address - region.base) as usize;
                region.data[offset..offset + src.len()].copy_from_slice(src);
                return Ok(());
            }
        }
        Err(MemoryError::UnmappedAddress(address))
    }

    /// Read raw bytes from guest memory at the specified address, checking permissions.
    pub fn read_bytes(&self, address: u64, dst: &mut [u8]) -> Result<(), MemoryError> {
        for region in &self.regions {
            if region.contains(address, dst.len()) {
                if !region.permissions.read {
                    return Err(MemoryError::PermissionViolation {
                        address,
                        required: "read",
                    });
                }
                let offset = (address - region.base) as usize;
                dst.copy_from_slice(&region.data[offset..offset + dst.len()]);
                return Ok(());
            }
        }
        Err(MemoryError::UnmappedAddress(address))
    }

    /// Read an instruction byte from guest memory, verifying execute permission.
    pub fn fetch_instruction_byte(&self, address: u64) -> Result<u8, MemoryError> {
        for region in &self.regions {
            if region.contains(address, 1) {
                if !region.permissions.execute {
                    return Err(MemoryError::PermissionViolation {
                        address,
                        required: "execute",
                    });
                }
                let offset = (address - region.base) as usize;
                return Ok(region.data[offset]);
            }
        }
        Err(MemoryError::UnmappedAddress(address))
    }

    /// Read a 64-bit integer (little endian) from guest memory.
    pub fn read_u64(&self, address: u64) -> Result<u64, MemoryError> {
        let mut buf = [0u8; 8];
        self.read_bytes(address, &mut buf)?;
        Ok(u64::from_le_bytes(buf))
    }

    /// Read a 64-bit integer ignoring region read permissions (e.g. loader/relocation phase).
    pub fn read_u64_bypass(&self, address: u64) -> Result<u64, MemoryError> {
        for region in &self.regions {
            if region.contains(address, 8) {
                let offset = (address - region.base) as usize;
                let buf: [u8; 8] = region.data[offset..offset + 8].try_into().unwrap();
                return Ok(u64::from_le_bytes(buf));
            }
        }
        Err(MemoryError::UnmappedAddress(address))
    }

    /// Write a 64-bit integer (little endian) to guest memory.
    pub fn write_u64(&mut self, address: u64, value: u64) -> Result<(), MemoryError> {
        self.write_bytes(address, &value.to_le_bytes())
    }

    /// Write a 64-bit integer ignoring region write permissions (e.g. loader/relocation phase).
    pub fn write_u64_bypass(&mut self, address: u64, value: u64) -> Result<(), MemoryError> {
        let bytes = value.to_le_bytes();
        for region in &mut self.regions {
            if region.contains(address, 8) {
                let offset = (address - region.base) as usize;
                region.data[offset..offset + 8].copy_from_slice(&bytes);
                return Ok(());
            }
        }
        Err(MemoryError::UnmappedAddress(address))
    }

    /// Read a 32-bit integer (little endian) from guest memory.
    pub fn read_u32(&self, address: u64) -> Result<u32, MemoryError> {
        let mut buf = [0u8; 4];
        self.read_bytes(address, &mut buf)?;
        Ok(u32::from_le_bytes(buf))
    }

    /// Write a 32-bit integer (little endian) to guest memory.
    pub fn write_u32(&mut self, address: u64, value: u32) -> Result<(), MemoryError> {
        self.write_bytes(address, &value.to_le_bytes())
    }

    /// Write a 32-bit integer ignoring region write permissions (e.g. loader/relocation phase).
    pub fn write_u32_bypass(&mut self, address: u64, value: u32) -> Result<(), MemoryError> {
        let bytes = value.to_le_bytes();
        for region in &mut self.regions {
            if region.contains(address, 4) {
                let offset = (address - region.base) as usize;
                region.data[offset..offset + 4].copy_from_slice(&bytes);
                return Ok(());
            }
        }
        Err(MemoryError::UnmappedAddress(address))
    }

    /// Read a single byte from guest memory.
    pub fn read_u8(&self, address: u64) -> Result<u8, MemoryError> {
        let mut buf = [0u8; 1];
        self.read_bytes(address, &mut buf)?;
        Ok(buf[0])
    }

    /// Write a single byte to guest memory.
    pub fn write_u8(&mut self, address: u64, value: u8) -> Result<(), MemoryError> {
        self.write_bytes(address, &[value])
    }

    /// Read a 16-bit little-endian integer from guest memory.
    pub fn read_u16(&self, address: u64) -> Result<u16, MemoryError> {
        let mut buf = [0u8; 2];
        self.read_bytes(address, &mut buf)?;
        Ok(u16::from_le_bytes(buf))
    }

    /// Write a 16-bit little-endian integer to guest memory.
    pub fn write_u16(&mut self, address: u64, value: u16) -> Result<(), MemoryError> {
        self.write_bytes(address, &value.to_le_bytes())
    }
}
