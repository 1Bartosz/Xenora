// SPDX-License-Identifier: GPL-3.0-or-later

//! W^X-compliant native executable JIT memory allocator for Apple Silicon.
//!
//! Uses macOS `MAP_JIT` with `pthread_jit_write_protect_np` and cache invalidation.

use std::fmt;
use std::ptr::NonNull;

unsafe extern "C" {
    fn pthread_jit_write_protect_np(enabled: i32);
    fn sys_dcache_flush(start: *mut libc::c_void, len: usize);
    fn sys_icache_invalidate(start: *mut libc::c_void, len: usize);
}

/// Errors relating to JIT memory allocation and protection.
#[derive(Debug, PartialEq, Eq)]
pub enum JitError {
    AllocationFailed(i32),
    CapacityExceeded,
    ProtectionFailed,
}

impl fmt::Display for JitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AllocationFailed(err) => write!(f, "mmap MAP_JIT failed with errno {err}"),
            Self::CapacityExceeded => write!(f, "JIT code cache capacity exceeded"),
            Self::ProtectionFailed => write!(f, "failed to alter JIT memory protection"),
        }
    }
}

impl std::error::Error for JitError {}

/// W^X-compliant JIT code buffer on macOS ARM64.
pub struct JitMemory {
    ptr: NonNull<u8>,
    capacity: usize,
    used: usize,
}

// JitMemory manages thread-independent JIT code memory (functions can be invoked across threads)
unsafe impl Send for JitMemory {}
unsafe impl Sync for JitMemory {}

impl JitMemory {
    /// Allocate a new JIT memory region using `MAP_JIT`.
    pub fn new(capacity: usize) -> Result<Self, JitError> {
        let page_size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
        let aligned_capacity = aligned_capacity(capacity, page_size)?;

        let ptr = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                aligned_capacity,
                libc::PROT_READ | libc::PROT_WRITE | libc::PROT_EXEC,
                libc::MAP_ANON | libc::MAP_PRIVATE | libc::MAP_JIT,
                -1,
                0,
            )
        };

        if ptr == libc::MAP_FAILED {
            let errno = unsafe { *libc::__error() };
            return Err(JitError::AllocationFailed(errno));
        }

        // Initialize to executable state by default (write-protected)
        unsafe {
            pthread_jit_write_protect_np(1);
        }

        Ok(Self {
            ptr: NonNull::new(ptr as *mut u8).unwrap(),
            capacity: aligned_capacity,
            used: 0,
        })
    }

    /// Base pointer of the JIT allocation.
    pub fn as_ptr(&self) -> *const u8 {
        self.ptr.as_ptr()
    }

    /// Total capacity in bytes.
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Currently committed bytes.
    pub fn used(&self) -> usize {
        self.used
    }

    /// Emit an ARM64 machine code block into the JIT memory.
    ///
    /// Automatically manages `pthread_jit_write_protect_np`, copies bytes,
    /// flushes data cache and invalidates instruction cache.
    pub fn emit_block(&mut self, instructions: &[u32]) -> Result<*const u8, JitError> {
        let bytes_len = emit_len(self.used, instructions.len(), self.capacity)?;

        let emit_offset = self.used;
        let dst_ptr = unsafe { self.ptr.as_ptr().add(emit_offset) };

        unsafe {
            // 1. Enable write permission for this thread
            pthread_jit_write_protect_np(0);

            // 2. Copy machine instructions
            std::ptr::copy_nonoverlapping(instructions.as_ptr() as *const u8, dst_ptr, bytes_len);

            // 3. Flush D-cache and invalidate I-cache
            sys_dcache_flush(dst_ptr as *mut libc::c_void, bytes_len);
            sys_icache_invalidate(dst_ptr as *mut libc::c_void, bytes_len);

            // 4. Restore write-protection (executable only)
            pthread_jit_write_protect_np(1);
        }

        self.used += bytes_len;
        Ok(dst_ptr as *const u8)
    }

    /// Clear the JIT cache buffer for reuse.
    pub fn clear(&mut self) {
        self.used = 0;
    }
}

/// Round `capacity` up to a whole number of pages, rejecting an invalid page size
/// (`sysconf` failure, zero or not a power of two) and arithmetic overflow.
fn aligned_capacity(capacity: usize, page_size: libc::c_long) -> Result<usize, JitError> {
    let page_size = usize::try_from(page_size)
        .ok()
        .filter(|p| p.is_power_of_two())
        .ok_or(JitError::AllocationFailed(libc::EINVAL))?;
    capacity
        .checked_add(page_size - 1)
        .map(|c| c & !(page_size - 1))
        .ok_or(JitError::CapacityExceeded)
}

/// Byte length of `count` 4-byte instructions, if they fit after `used` bytes.
fn emit_len(used: usize, count: usize, capacity: usize) -> Result<usize, JitError> {
    count
        .checked_mul(4)
        .filter(|len| used.checked_add(*len).is_some_and(|end| end <= capacity))
        .ok_or(JitError::CapacityExceeded)
}

impl Drop for JitMemory {
    fn drop(&mut self) {
        unsafe {
            libc::munmap(self.ptr.as_ptr() as *mut libc::c_void, self.capacity);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capacity_rounds_up_to_whole_pages() {
        assert_eq!(aligned_capacity(1, 16384), Ok(16384));
        assert_eq!(aligned_capacity(16384, 16384), Ok(16384));
        assert_eq!(aligned_capacity(16385, 16384), Ok(32768));
        assert_eq!(aligned_capacity(0, 4096), Ok(0));
    }

    #[test]
    fn invalid_page_size_is_rejected() {
        for page in [-1, 0, 3, 12288] {
            assert_eq!(
                aligned_capacity(4096, page),
                Err(JitError::AllocationFailed(libc::EINVAL)),
                "page size {page}"
            );
        }
    }

    #[test]
    fn capacity_rounding_overflow_is_rejected() {
        assert_eq!(
            aligned_capacity(usize::MAX, 16384),
            Err(JitError::CapacityExceeded)
        );
        assert_eq!(
            aligned_capacity(usize::MAX - 16382, 16384),
            Err(JitError::CapacityExceeded)
        );
    }

    #[test]
    fn emit_length_is_checked() {
        assert_eq!(emit_len(0, 4, 16), Ok(16));
        assert_eq!(emit_len(4, 3, 16), Ok(12));
        assert_eq!(emit_len(4, 4, 16), Err(JitError::CapacityExceeded));
        // count * 4 overflows usize.
        assert_eq!(
            emit_len(0, usize::MAX / 2, usize::MAX),
            Err(JitError::CapacityExceeded)
        );
        // used + len overflows usize.
        assert_eq!(
            emit_len(usize::MAX - 3, 2, usize::MAX),
            Err(JitError::CapacityExceeded)
        );
    }

    #[test]
    fn emit_past_capacity_fails_and_leaves_usage_unchanged() {
        let mut jit = JitMemory::new(1).expect("MAP_JIT allocation");
        let words = jit.capacity() / 4;
        assert!(jit.emit_block(&vec![0xD503_201F; words]).is_ok()); // NOP fills the page
        assert_eq!(
            jit.emit_block(&[0xD503_201F]),
            Err(JitError::CapacityExceeded)
        );
        assert_eq!(jit.used(), jit.capacity());
    }
}
