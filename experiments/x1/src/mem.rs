// SPDX-License-Identifier: GPL-3.0-or-later
//! Guest virtual memory abstraction.
//!
//! # Design
//!
//! The guest address space is modelled as a list of non-overlapping [`Region`]s
//! each with its own permissions and backing storage.  All guest addresses are
//! 64-bit values; host pointers are never exposed to guest code or services.
//!
//! # Permissions
//!
//! Every region carries a [`Perm`] bitmask.  Loads require `READ`, stores
//! require `WRITE`, and instruction fetches require `EXEC`.  Accesses that
//! violate the permission of the target region (or hit an unmapped address)
//! return `Err(MemFault::…)` without modifying any state.
//!
//! # Cross-boundary access
//!
//! An access that spans two regions — even if both have the right permission —
//! is rejected with `Err(MemFault::CrossBoundary)` by the synthetic runtime.
//!
//! # Stack
//!
//! [`GuestMemory::alloc_stack`] reserves an aligned stack region.  The initial
//! RSP starts at the 16-byte-aligned top of the stack. The interpreter pushes
//! the eight-byte root return slot before guest entry.
//! Stack pages are readable and writable but not executable.
//!
//! # Supported integer widths
//!
//! Loads and stores are provided for `u8`, `u16`, `u32`, and `u64` in
//! little-endian byte order, as required by x86-64.

/// Permission bits for a mapped region.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Perm(u8);

impl Perm {
    pub(crate) const READ: Perm = Perm(1);
    pub(crate) const WRITE: Perm = Perm(2);
    pub(crate) const EXEC: Perm = Perm(4);

    pub(crate) const RX: Perm = Perm(1 | 4);
    pub(crate) const RW: Perm = Perm(1 | 2);

    #[inline]
    pub(crate) fn has(self, required: Perm) -> bool {
        self.0 & required.0 == required.0
    }
}

/// Reason an access was denied.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MemFault {
    /// The guest address is not mapped.
    Unmapped,
    /// The region is not readable.
    NotReadable,
    /// The region is not writable.
    NotWritable,
    /// The region is not executable.
    NotExecutable,
    /// The access straddles a region boundary.
    CrossBoundary,
    /// A stack-pointer overflow or underflow was detected.
    StackOverflow,
}

/// One contiguous mapped region.
#[derive(Debug)]
struct Region {
    /// Guest virtual address of the first byte.
    base: u64,
    /// Permission bits.
    perm: Perm,
    /// Backing storage, length equal to the mapped size.
    data: Vec<u8>,
}

impl Region {
    /// Guest virtual address one past the last byte of this region.
    #[inline]
    fn end(&self) -> u64 {
        self.base + self.data.len() as u64
    }

    /// Returns true if `[addr, addr+size)` is entirely within this region.
    fn contains(&self, addr: u64, size: usize) -> bool {
        if size == 0 {
            return addr >= self.base && addr <= self.end();
        }
        addr >= self.base
            && addr
                .checked_add(size as u64)
                .is_some_and(|end| end <= self.end())
    }

    /// Offset of `addr` within this region (unchecked).
    #[inline]
    fn offset(&self, addr: u64) -> usize {
        (addr - self.base) as usize
    }
}

/// The guest virtual address space.
///
/// Regions must not overlap; `map` enforces this.
#[derive(Debug)]
pub(crate) struct GuestMemory {
    regions: Vec<Region>,
    stack: Option<std::ops::Range<u64>>,
}

impl GuestMemory {
    /// Create an empty address space.
    pub(crate) fn new() -> Self {
        Self {
            regions: Vec::new(),
            stack: None,
        }
    }

    /// Map a new region starting at `base` with `size` bytes and `perm`.
    ///
    /// All bytes are initialised to `fill` (use `0` for zero-init, `0xA5` for
    /// sentinel fill in tests).
    ///
    /// # Errors
    ///
    /// Returns `false` if the range overlaps any existing mapping, `base +
    /// size` overflows `u64`, or `size` is zero.
    pub(crate) fn map(&mut self, base: u64, size: usize, perm: Perm, fill: u8) -> bool {
        if size == 0 {
            return false;
        }
        let end = match base.checked_add(size as u64) {
            Some(e) => e,
            None => return false,
        };
        // Check for overlap with every existing region.
        for r in &self.regions {
            // Regions [a,b) and [c,d) overlap iff a < d && c < b.
            if base < r.end() && r.base < end {
                return false;
            }
        }
        self.regions.push(Region {
            base,
            perm,
            data: vec![fill; size],
        });
        true
    }

    /// Check that `[addr, addr+size)` fits entirely within one region and
    /// that region has `required` permission.  Returns a reference to that
    /// region on success.
    fn check(&self, addr: u64, size: usize, required: Perm) -> Result<&Region, MemFault> {
        // Find the region that contains the first byte.
        let r = self
            .regions
            .iter()
            .find(|r| r.contains(addr, 1))
            .ok_or(MemFault::Unmapped)?;
        // The entire access must fit within the same region.
        if !r.contains(addr, size) {
            return Err(MemFault::CrossBoundary);
        }
        if required.has(Perm::READ) && !r.perm.has(Perm::READ) {
            return Err(MemFault::NotReadable);
        }
        if required.has(Perm::WRITE) && !r.perm.has(Perm::WRITE) {
            return Err(MemFault::NotWritable);
        }
        if required.has(Perm::EXEC) && !r.perm.has(Perm::EXEC) {
            return Err(MemFault::NotExecutable);
        }
        Ok(r)
    }

    fn check_mut(
        &mut self,
        addr: u64,
        size: usize,
        required: Perm,
    ) -> Result<&mut Region, MemFault> {
        // Two-pass: first find the index, then borrow mutably.
        let idx = self
            .regions
            .iter()
            .position(|r| r.contains(addr, 1))
            .ok_or(MemFault::Unmapped)?;
        let r = &mut self.regions[idx];
        if !r.contains(addr, size) {
            return Err(MemFault::CrossBoundary);
        }
        if required.has(Perm::READ) && !r.perm.has(Perm::READ) {
            return Err(MemFault::NotReadable);
        }
        if required.has(Perm::WRITE) && !r.perm.has(Perm::WRITE) {
            return Err(MemFault::NotWritable);
        }
        if required.has(Perm::EXEC) && !r.perm.has(Perm::EXEC) {
            return Err(MemFault::NotExecutable);
        }
        Ok(r)
    }

    // ── loads ────────────────────────────────────────────────────────────────

    /// Load a byte from a readable guest address.
    #[cfg(test)]
    pub(crate) fn load_u8(&self, addr: u64) -> Result<u8, MemFault> {
        let r = self.check(addr, 1, Perm::READ)?;
        Ok(r.data[r.offset(addr)])
    }

    /// Load a little-endian u16.
    #[cfg(test)]
    pub(crate) fn load_u16(&self, addr: u64) -> Result<u16, MemFault> {
        let r = self.check(addr, 2, Perm::READ)?;
        let off = r.offset(addr);
        Ok(u16::from_le_bytes(r.data[off..off + 2].try_into().unwrap()))
    }

    /// Load a little-endian u32.
    pub(crate) fn load_u32(&self, addr: u64) -> Result<u32, MemFault> {
        let r = self.check(addr, 4, Perm::READ)?;
        let off = r.offset(addr);
        Ok(u32::from_le_bytes(r.data[off..off + 4].try_into().unwrap()))
    }

    /// Load a little-endian u64.
    pub(crate) fn load_u64(&self, addr: u64) -> Result<u64, MemFault> {
        let r = self.check(addr, 8, Perm::READ)?;
        let off = r.offset(addr);
        Ok(u64::from_le_bytes(r.data[off..off + 8].try_into().unwrap()))
    }

    // ── stores ───────────────────────────────────────────────────────────────

    /// Store a byte to a writable guest address.
    #[cfg(test)]
    pub(crate) fn store_u8(&mut self, addr: u64, value: u8) -> Result<(), MemFault> {
        let r = self.check_mut(addr, 1, Perm::WRITE)?;
        let off = r.offset(addr);
        r.data[off] = value;
        Ok(())
    }

    /// Store a little-endian u16.
    #[cfg(test)]
    pub(crate) fn store_u16(&mut self, addr: u64, value: u16) -> Result<(), MemFault> {
        let r = self.check_mut(addr, 2, Perm::WRITE)?;
        let off = r.offset(addr);
        r.data[off..off + 2].copy_from_slice(&value.to_le_bytes());
        Ok(())
    }

    /// Store a little-endian u32.
    pub(crate) fn store_u32(&mut self, addr: u64, value: u32) -> Result<(), MemFault> {
        let r = self.check_mut(addr, 4, Perm::WRITE)?;
        let off = r.offset(addr);
        r.data[off..off + 4].copy_from_slice(&value.to_le_bytes());
        Ok(())
    }

    /// Store a little-endian u64.
    pub(crate) fn store_u64(&mut self, addr: u64, value: u64) -> Result<(), MemFault> {
        let r = self.check_mut(addr, 8, Perm::WRITE)?;
        let off = r.offset(addr);
        r.data[off..off + 8].copy_from_slice(&value.to_le_bytes());
        Ok(())
    }

    // ── instruction fetch ────────────────────────────────────────────────────

    /// Borrow the slice `[addr, addr+max_len)` from an executable region.
    ///
    /// Returns the longest contiguous slice starting at `addr` within the
    /// region, up to `max_len` bytes.  Returns `Err(MemFault::Unmapped)` or
    /// `Err(MemFault::NotExecutable)` on failure.
    pub(crate) fn fetch(&self, addr: u64, max_len: usize) -> Result<&[u8], MemFault> {
        let r = self
            .regions
            .iter()
            .find(|r| r.contains(addr, 1))
            .ok_or(MemFault::Unmapped)?;
        if !r.perm.has(Perm::EXEC) {
            return Err(MemFault::NotExecutable);
        }
        let off = r.offset(addr);
        let avail = r.data.len() - off;
        Ok(&r.data[off..off + avail.min(max_len)])
    }

    // ── stack ────────────────────────────────────────────────────────────────

    /// Allocate one guest stack. The interpreter pushes the initial return
    /// slot from the returned 16-byte-aligned top.
    pub(crate) fn alloc_stack(&mut self, base: u64, size: usize) -> Option<u64> {
        let top = base.checked_add(size as u64)?;
        if self.stack.is_some()
            || size < 16
            || !top.is_multiple_of(16)
            || !self.map(base, size, Perm::RW, 0)
        {
            return None;
        }
        self.stack = Some(base..top);
        Some(top)
    }

    fn check_stack_slot(&self, slot: u64) -> Result<(), MemFault> {
        let stack = self.stack.as_ref().ok_or(MemFault::StackOverflow)?;
        if slot >= stack.start && slot <= stack.end - 8 {
            Ok(())
        } else {
            Err(MemFault::StackOverflow)
        }
    }

    pub(crate) fn check_import_frame(&self, rsp: u64) -> Result<(), MemFault> {
        // The synthetic x64 service requires an aligned call site, space for
        // the return address, and the four eight-byte home slots.
        if !rsp.is_multiple_of(16) {
            return Err(MemFault::StackOverflow);
        }
        self.check_stack_slot(rsp.checked_sub(8).ok_or(MemFault::StackOverflow)?)?;
        self.check_stack_slot(rsp.checked_add(24).ok_or(MemFault::StackOverflow)?)
    }

    /// Push a u64 onto the guest stack (RSP -= 8, then write).
    ///
    /// Returns the new RSP, or `Err(MemFault::StackOverflow)` if the
    /// decremented RSP would underflow or the write fails.
    pub(crate) fn push_u64(&mut self, rsp: u64, value: u64) -> Result<u64, MemFault> {
        let new_rsp = rsp.checked_sub(8).ok_or(MemFault::StackOverflow)?;
        self.check_stack_slot(new_rsp)?;
        self.store_u64(new_rsp, value)?;
        Ok(new_rsp)
    }

    /// Pop a u64 from the guest stack (read, then RSP += 8).
    ///
    /// Returns `(value, new_rsp)`, or `Err(MemFault)` on read failure or
    /// overflow.
    pub(crate) fn pop_u64(&mut self, rsp: u64) -> Result<(u64, u64), MemFault> {
        self.check_stack_slot(rsp)?;
        let value = self.load_u64(rsp)?;
        let new_rsp = rsp.checked_add(8).ok_or(MemFault::StackOverflow)?;
        Ok((value, new_rsp))
    }

    /// Read a contiguous byte slice from guest memory for inspection (tests,
    /// CLI output).  Requires READ permission.
    pub(crate) fn read_bytes(&self, addr: u64, len: usize) -> Result<Vec<u8>, MemFault> {
        let r = self.check(addr, len, Perm::READ)?;
        let off = r.offset(addr);
        Ok(r.data[off..off + len].to_vec())
    }

    /// Install image bytes before exposing the memory to the interpreter.
    pub(crate) fn initialize(&mut self, addr: u64, src: &[u8]) -> Result<(), MemFault> {
        let region = self
            .regions
            .iter_mut()
            .find(|r| r.contains(addr, src.len()))
            .ok_or(MemFault::Unmapped)?;
        let offset = region.offset(addr);
        region.data[offset..offset + src.len()].copy_from_slice(src);
        Ok(())
    }

    pub(crate) fn read_readonly(&self, addr: u64, len: usize) -> Result<&[u8], MemFault> {
        let region = self.check(addr, len, Perm::READ)?;
        if region.perm != Perm::READ {
            return Err(MemFault::NotReadable);
        }
        let offset = region.offset(addr);
        Ok(&region.data[offset..offset + len])
    }

    #[cfg(feature = "metal")]
    pub(crate) fn check_writable(&self, addr: u64, len: usize) -> Result<(), MemFault> {
        self.check(addr, len, Perm::WRITE).map(|_| ())
    }

    /// Write a byte slice into guest memory.  Requires WRITE permission.
    pub(crate) fn write_bytes(&mut self, addr: u64, src: &[u8]) -> Result<(), MemFault> {
        let r = self.check_mut(addr, src.len(), Perm::WRITE)?;
        let off = r.offset(addr);
        r.data[off..off + src.len()].copy_from_slice(src);
        Ok(())
    }
}

// ── tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn rw_map() -> GuestMemory {
        let mut mem = GuestMemory::new();
        assert!(mem.map(0x1000, 0x1000, Perm::RW, 0));
        mem
    }

    #[test]
    fn load_store_roundtrip_all_widths() {
        let mut mem = rw_map();
        mem.store_u8(0x1000, 0xAB).unwrap();
        mem.store_u16(0x1001, 0xCDEF).unwrap();
        mem.store_u32(0x1003, 0x12345678).unwrap();
        mem.store_u64(0x1007, 0xDEAD_BEEF_CAFE_BABE).unwrap();
        assert_eq!(mem.load_u8(0x1000).unwrap(), 0xAB);
        assert_eq!(mem.load_u16(0x1001).unwrap(), 0xCDEF);
        assert_eq!(mem.load_u32(0x1003).unwrap(), 0x12345678);
        assert_eq!(mem.load_u64(0x1007).unwrap(), 0xDEAD_BEEF_CAFE_BABE);
    }

    #[test]
    fn unmapped_address_returns_unmapped() {
        let mem = rw_map();
        assert_eq!(mem.load_u8(0x0FFF).unwrap_err(), MemFault::Unmapped);
        assert_eq!(mem.load_u8(0x2000).unwrap_err(), MemFault::Unmapped);
    }

    #[test]
    fn permission_denied_errors() {
        let mut mem = GuestMemory::new();
        assert!(mem.map(0x1000, 0x1000, Perm::READ, 0)); // read-only
        assert_eq!(mem.store_u8(0x1000, 1).unwrap_err(), MemFault::NotWritable);
        assert_eq!(mem.fetch(0x1000, 4).unwrap_err(), MemFault::NotExecutable);
    }

    #[test]
    fn cross_boundary_rejected() {
        let mut mem = GuestMemory::new();
        assert!(mem.map(0x1000, 4, Perm::RW, 0));
        assert!(mem.map(0x1004, 4, Perm::RW, 0));
        // A u64 spanning both regions must fail.
        assert_eq!(mem.load_u64(0x1002).unwrap_err(), MemFault::CrossBoundary);
        assert_eq!(
            mem.store_u64(0x1002, 0).unwrap_err(),
            MemFault::CrossBoundary
        );
    }

    #[test]
    fn overlap_map_fails() {
        let mut mem = GuestMemory::new();
        assert!(mem.map(0x1000, 0x1000, Perm::RW, 0));
        assert!(!mem.map(0x1000, 0x1000, Perm::RW, 0)); // exact overlap
        assert!(!mem.map(0x0FFF, 2, Perm::RW, 0)); // partial overlap start
        assert!(!mem.map(0x1FFF, 2, Perm::RW, 0)); // partial overlap end
        assert!(!mem.map(0x0800, 0x2000, Perm::RW, 0)); // superset
    }

    #[test]
    fn zero_size_map_fails() {
        let mut mem = GuestMemory::new();
        assert!(!mem.map(0x1000, 0, Perm::RW, 0));
    }

    #[test]
    fn stack_push_pop_roundtrip() {
        let mut mem = GuestMemory::new();
        let rsp = mem.alloc_stack(0x7000_0000, 0x1000).unwrap();
        // Initial RSP must be within the region.
        assert_eq!(rsp, 0x7000_1000);
        let new_rsp = mem.push_u64(rsp, 0xDEAD_CAFE).unwrap();
        assert_eq!(new_rsp, rsp - 8);
        let (val, restored) = mem.pop_u64(new_rsp).unwrap();
        assert_eq!(val, 0xDEAD_CAFE);
        assert_eq!(restored, rsp);
    }

    #[test]
    fn stack_overflow_on_underflow() {
        let mut mem = GuestMemory::new();
        // RSP at the bottom of stack — one more push should fail.
        let _ = mem.alloc_stack(0x1000, 0x10);
        let bottom = 0x1000u64;
        // Store into the bottom 8 bytes first (valid).
        mem.store_u64(bottom, 0).unwrap();
        // push_u64 at address 0 would underflow.
        assert_eq!(mem.push_u64(0, 0).unwrap_err(), MemFault::StackOverflow);
    }

    #[test]
    fn stack_guard_rejects_push_and_pop_even_in_adjacent_writable_data() {
        let mut mem = GuestMemory::new();
        assert!(mem.map(0x0FF0, 16, Perm::RW, 0xA5));
        assert_eq!(mem.alloc_stack(0x1000, 16), Some(0x1010));
        assert_eq!(mem.push_u64(0x1000, 0), Err(MemFault::StackOverflow));
        assert_eq!(mem.pop_u64(0x0FF8), Err(MemFault::StackOverflow));
        assert_eq!(mem.load_u64(0x0FF8), Ok(0xA5A5_A5A5_A5A5_A5A5));
    }

    #[test]
    fn import_frame_requires_alignment_and_full_home_space() {
        let mut mem = GuestMemory::new();
        assert_eq!(mem.alloc_stack(0x1000, 0x100), Some(0x1100));
        assert_eq!(mem.check_import_frame(0x10D0), Ok(()));
        for rsp in [0x1000, 0x10D8, 0x10F0, u64::MAX - 15] {
            assert_eq!(mem.check_import_frame(rsp), Err(MemFault::StackOverflow));
        }
    }

    #[test]
    fn exec_fetch_returns_slice() {
        let mut mem = GuestMemory::new();
        assert!(mem.map(0x2000, 16, Perm::RX, 0xCC));
        let slice = mem.fetch(0x2004, 8).unwrap();
        assert_eq!(slice.len(), 8);
        assert!(slice.iter().all(|&b| b == 0xCC));
    }

    #[test]
    fn exec_fetch_at_end_returns_shorter_slice() {
        let mut mem = GuestMemory::new();
        assert!(mem.map(0x2000, 4, Perm::RX, 0x90));
        let slice = mem.fetch(0x2002, 100).unwrap();
        assert_eq!(slice.len(), 2);
    }

    #[test]
    fn read_write_bytes() {
        let mut mem = rw_map();
        mem.write_bytes(0x1000, b"hello").unwrap();
        assert_eq!(mem.read_bytes(0x1000, 5).unwrap(), b"hello");
    }

    #[test]
    fn fill_sentinel_preserved() {
        let mut mem = GuestMemory::new();
        assert!(mem.map(0x3000, 8, Perm::RW, 0xA5));
        assert_eq!(mem.load_u64(0x3000).unwrap(), 0xA5A5_A5A5_A5A5_A5A5);
    }
}
