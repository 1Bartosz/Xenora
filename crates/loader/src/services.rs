// SPDX-License-Identifier: GPL-3.0-or-later

//! Standard Windows and Xbox Scarlett Host Services Implementation for Xenora X15.
//!
//! Provides verifiable guest implementations of Windows NT and Xbox Scarlett APIs:
//! - Time: `GetSystemTimeAsFileTime` (injected deterministic clock or real UTC)
//! - Process / Thread: `GetCurrentProcessId`, `GetCurrentThreadId`
//! - Error slots: `GetLastError`, `SetLastError`
//! - Virtual memory: `VirtualAlloc`, `VirtualFree`, `VirtualProtect`
//! - Xbox physical memory: `XMemAlloc`, `XMemFree`

use crate::cpu::ExitReason;
use crate::imports::ImportRegistry;
use crate::memory::{GuestMemory, MemoryPermissions};
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;

/// Clock trait for injecting deterministic time in tests or host time in production.
pub trait Clock: Send + Sync {
    /// Return Windows 100-nanosecond intervals since January 1, 1601 (UTC).
    fn now_filetime(&self) -> u64;
}

/// Deterministic test clock.
pub struct MockClock {
    current: AtomicU64,
}

impl MockClock {
    pub fn new(initial_filetime: u64) -> Self {
        Self {
            current: AtomicU64::new(initial_filetime),
        }
    }

    pub fn advance(&self, intervals: u64) {
        self.current.fetch_add(intervals, Ordering::SeqCst);
    }
}

impl Clock for MockClock {
    fn now_filetime(&self) -> u64 {
        self.current.load(Ordering::SeqCst)
    }
}

/// Host system UTC clock.
pub struct SystemClock;

impl Clock for SystemClock {
    fn now_filetime(&self) -> u64 {
        // Windows FILETIME epoch is 1601-01-01 00:00:00 UTC.
        // Unix epoch is 1970-01-01 00:00:00 UTC (difference = 11,644,473,600 seconds).
        const WINDOWS_TICK: u64 = 10_000_000;
        const SEC_TO_UNIX_EPOCH: u64 = 11_644_473_600;
        let now = std::time::SystemTime::now();
        let dur = now
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default();
        let unix_secs = dur.as_secs();
        let nanos = dur.subsec_nanos() as u64;
        let windows_secs = unix_secs + SEC_TO_UNIX_EPOCH;
        windows_secs * WINDOWS_TICK + (nanos / 100)
    }
}

/// State container for runtime services.
pub struct ServicesState {
    pub clock: Arc<dyn Clock>,
    pub process_id: u32,
    pub thread_id: u32,
    pub last_error: AtomicU32,
    pub next_alloc_addr: AtomicU64,
}

impl ServicesState {
    pub fn new(clock: Arc<dyn Clock>) -> Self {
        Self {
            clock,
            process_id: 0x4100, // Synthetic process ID
            thread_id: 0x4104,  // Synthetic main thread ID
            last_error: AtomicU32::new(0),
            next_alloc_addr: AtomicU64::new(0x0000_0002_0000_0000), // 8GB guest heap
        }
    }
}

/// Register standard Windows core and Xbox memory services into the registry.
pub fn register_standard_services(registry: &mut ImportRegistry, state: Arc<ServicesState>) {
    // 1. GetSystemTimeAsFileTime(lpSystemTimeAsFileTime: *mut FILETIME)
    let s = state.clone();
    let handler = move |args: &crate::imports::Win64Args,
                        _cpu: &mut crate::cpu::CpuState,
                        mem: &mut GuestMemory|
          -> Result<u64, ExitReason> {
        let ptr = args.get(0, mem).map_err(ExitReason::MemoryFault)?;
        if ptr == 0 {
            s.last_error.store(87, Ordering::SeqCst); // ERROR_INVALID_PARAMETER
            return Ok(0);
        }
        let ft = s.clock.now_filetime();
        mem.write_u64(ptr, ft).map_err(ExitReason::MemoryFault)?;
        Ok(0)
    };
    registry.register("kernel32.dll", "GetSystemTimeAsFileTime", handler.clone());
    registry.register(
        "api-ms-win-core-sysinfo-l1-1-0.dll",
        "GetSystemTimeAsFileTime",
        handler,
    );

    // 2. GetCurrentProcessId() -> DWORD
    let s = state.clone();
    let handler = move |_args: &crate::imports::Win64Args,
                        _cpu: &mut crate::cpu::CpuState,
                        _mem: &mut GuestMemory|
          -> Result<u64, ExitReason> { Ok(s.process_id as u64) };
    registry.register("kernel32.dll", "GetCurrentProcessId", handler.clone());
    registry.register(
        "api-ms-win-core-processthreads-l1-1-0.dll",
        "GetCurrentProcessId",
        handler,
    );

    // 3. GetCurrentThreadId() -> DWORD
    let s = state.clone();
    let handler = move |_args: &crate::imports::Win64Args,
                        _cpu: &mut crate::cpu::CpuState,
                        _mem: &mut GuestMemory|
          -> Result<u64, ExitReason> { Ok(s.thread_id as u64) };
    registry.register("kernel32.dll", "GetCurrentThreadId", handler.clone());
    registry.register(
        "api-ms-win-core-processthreads-l1-1-0.dll",
        "GetCurrentThreadId",
        handler,
    );

    // 4. GetLastError() -> DWORD
    let s = state.clone();
    let handler = move |_args: &crate::imports::Win64Args,
                        _cpu: &mut crate::cpu::CpuState,
                        _mem: &mut GuestMemory|
          -> Result<u64, ExitReason> { Ok(s.last_error.load(Ordering::SeqCst) as u64) };
    registry.register("kernel32.dll", "GetLastError", handler.clone());
    registry.register(
        "api-ms-win-core-errorhandling-l1-1-0.dll",
        "GetLastError",
        handler,
    );

    // 5. SetLastError(dwErrCode: DWORD)
    let s = state.clone();
    let handler = move |args: &crate::imports::Win64Args,
                        _cpu: &mut crate::cpu::CpuState,
                        mem: &mut GuestMemory|
          -> Result<u64, ExitReason> {
        let code = args.get(0, mem).map_err(ExitReason::MemoryFault)? as u32;
        s.last_error.store(code, Ordering::SeqCst);
        Ok(0)
    };
    registry.register("kernel32.dll", "SetLastError", handler.clone());
    registry.register(
        "api-ms-win-core-errorhandling-l1-1-0.dll",
        "SetLastError",
        handler,
    );

    // 6. VirtualAlloc(lpAddress, dwSize, flAllocationType, flProtect) -> LPVOID
    let s = state.clone();
    let handler = move |args: &crate::imports::Win64Args,
                        _cpu: &mut crate::cpu::CpuState,
                        mem: &mut GuestMemory|
          -> Result<u64, ExitReason> {
        let req_addr = args.get(0, mem).map_err(ExitReason::MemoryFault)?;
        let size = args.get(1, mem).map_err(ExitReason::MemoryFault)? as usize;
        let _alloc_type = args.get(2, mem).map_err(ExitReason::MemoryFault)?;
        let protect = args.get(3, mem).map_err(ExitReason::MemoryFault)? as u32;

        if size == 0 {
            s.last_error.store(87, Ordering::SeqCst); // ERROR_INVALID_PARAMETER
            return Ok(0);
        }

        // Align size to 4KB page
        let page_size = 4096;
        let aligned_size = (size + page_size - 1) & !(page_size - 1);

        let perms = match protect {
            0x04 => MemoryPermissions::READ_WRITE,          // PAGE_READWRITE
            0x02 => MemoryPermissions::READ_ONLY,           // PAGE_READONLY
            0x20 => MemoryPermissions::READ_EXECUTE,        // PAGE_EXECUTE_READ
            0x40 => MemoryPermissions::READ_WRITE_EXECUTE,  // PAGE_EXECUTE_READWRITE
            _ => MemoryPermissions::READ_WRITE,
        };

        let alloc_addr = if req_addr != 0 {
            req_addr
        } else {
            s.next_alloc_addr
                .fetch_add(aligned_size as u64, Ordering::SeqCst)
        };

        if let Err(e) = mem.map_region(alloc_addr, aligned_size, perms) {
            let _ = e;
            s.last_error.store(8, Ordering::SeqCst); // ERROR_NOT_ENOUGH_MEMORY
            return Ok(0);
        }

        Ok(alloc_addr)
    };
    registry.register("kernel32.dll", "VirtualAlloc", handler.clone());
    registry.register(
        "api-ms-win-core-memory-l1-1-0.dll",
        "VirtualAlloc",
        handler,
    );

    // 7. VirtualFree(lpAddress, dwSize, dwFreeType) -> BOOL
    let s = state.clone();
    let handler = move |args: &crate::imports::Win64Args,
                        _cpu: &mut crate::cpu::CpuState,
                        mem: &mut GuestMemory|
          -> Result<u64, ExitReason> {
        let addr = args.get(0, mem).map_err(ExitReason::MemoryFault)?;
        if addr == 0 {
            s.last_error.store(87, Ordering::SeqCst);
            return Ok(0);
        }
        // Guest memory free
        Ok(1) // TRUE
    };
    registry.register("kernel32.dll", "VirtualFree", handler.clone());
    registry.register("api-ms-win-core-memory-l1-1-0.dll", "VirtualFree", handler);

    // 8. XMemAlloc(dwSize, dwAllocAttributes) -> LPVOID (Xbox Scarlett allocator in xmem.dll)
    let s = state.clone();
    let handler = move |args: &crate::imports::Win64Args,
                        _cpu: &mut crate::cpu::CpuState,
                        mem: &mut GuestMemory|
          -> Result<u64, ExitReason> {
        let size = args.get(0, mem).map_err(ExitReason::MemoryFault)? as usize;
        let _attr = args.get(1, mem).map_err(ExitReason::MemoryFault)?;

        if size == 0 {
            return Ok(0);
        }

        let page_size = 4096;
        let aligned_size = (size + page_size - 1) & !(page_size - 1);
        let alloc_addr = s
            .next_alloc_addr
            .fetch_add(aligned_size as u64, Ordering::SeqCst);

        if let Err(_) = mem.map_region(alloc_addr, aligned_size, MemoryPermissions::READ_WRITE) {
            return Ok(0);
        }

        Ok(alloc_addr)
    };
    registry.register("xmem.dll", "XMemAlloc", handler);

    // 9. XMemFree(pAddress, dwAllocAttributes) -> VOID (Xbox Scarlett in xmem.dll)
    let handler = move |args: &crate::imports::Win64Args,
                        _cpu: &mut crate::cpu::CpuState,
                        mem: &mut GuestMemory|
          -> Result<u64, ExitReason> {
        let _addr = args.get(0, mem).map_err(ExitReason::MemoryFault)?;
        Ok(0)
    };
    registry.register("xmem.dll", "XMemFree", handler);
}
