// SPDX-License-Identifier: GPL-3.0-or-later

//! Standard Windows and Xbox Scarlett Host Services Implementation for Xenora X15.
//!
//! Provides verifiable guest implementations of Windows NT and Xbox Scarlett APIs:
//! - Time: UTC file time and a monotonic performance counter
//! - Process and thread identity and guest process priority boost policy
//! - Error slots: `GetLastError`, `SetLastError`
//! - Virtual memory: `VirtualAlloc`, `VirtualFree`
//! - Xbox physical memory: `XMemAlloc`, `XMemFree`

mod crt;
mod events;
mod heap;

use crate::cpu::ExitReason;
use crate::imports::{ImportRegistry, ServiceStatus};
use crate::memory::{ALLOCATION_GRANULARITY, GuestMemory, MemoryError, MemoryPermissions};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;

const CURRENT_PROCESS: u64 = u64::MAX;
const CURRENT_THREAD: u64 = u64::MAX - 1;
const ERROR_ACCESS_DENIED: u32 = 5;
const ERROR_INVALID_HANDLE: u32 = 6;
const ERROR_NOT_ENOUGH_MEMORY: u32 = 8;
const ERROR_NOT_SUPPORTED: u32 = 50;
const ERROR_INVALID_PARAMETER: u32 = 87;
const ERROR_INVALID_ADDRESS: u32 = 487;
const THREAD_QUERY_RIGHTS: u32 = 0x0040 | 0x0800;
const THREAD_SET_RIGHTS: u32 = 0x0020 | 0x0400;
const THREAD_SUPPORTED_RIGHTS: u32 = THREAD_QUERY_RIGHTS | THREAD_SET_RIGHTS | 0x0010_0000;
const THREAD_PRIORITY_ERROR_RETURN: u64 = 0x7fff_ffff;
const MAX_OBJECT_HANDLES: usize = 1024;
const DUPLICATE_SAME_ACCESS: u32 = 2;
const HANDLE_API: &str = "api-ms-win-core-handle-l1-1-0.dll";
const PAGE_SIZE: u64 = 4096;
const MEM_COMMIT_RESERVE: u32 = 0x3000;
const MEM_RELEASE: u32 = 0x8000;
const ERROR_NOACCESS: u32 = 998;
const KERNEL32: &str = "kernel32.dll";
const SYSINFO_API: &str = "api-ms-win-core-sysinfo-l1-1-0.dll";
const PROCESS_THREADS_V1: &str = "api-ms-win-core-processthreads-l1-1-0.dll";
const PROCESS_THREADS_V2: &str = "api-ms-win-core-processthreads-l1-1-2.dll";

const PERFORMANCE_FREQUENCY: u64 = 10_000_000;
const MAX_PERFORMANCE_COUNT: u64 = i64::MAX as u64;

/// Clock trait for injecting deterministic time in tests or host time in production.
pub trait Clock: Send + Sync {
    /// Return Windows 100-nanosecond intervals since January 1, 1601 (UTC).
    fn now_filetime(&self) -> u64;

    /// Return elapsed monotonic time in 100-nanosecond ticks, capped at `i64::MAX`.
    fn performance_count(&self) -> u64 {
        static ORIGIN: OnceLock<Instant> = OnceLock::new();
        let ticks = ORIGIN.get_or_init(Instant::now).elapsed().as_nanos() / 100;
        ticks.min(u128::from(MAX_PERFORMANCE_COUNT)) as u64
    }
}

/// Deterministic test clock.
pub struct MockClock {
    current: AtomicU64,
    performance_count: AtomicU64,
}

impl MockClock {
    pub fn new(initial_filetime: u64) -> Self {
        Self {
            current: AtomicU64::new(initial_filetime),
            performance_count: AtomicU64::new(0),
        }
    }

    pub fn advance(&self, intervals: u64) {
        self.current.fetch_add(intervals, Ordering::SeqCst);
        self.performance_count
            .try_update(Ordering::SeqCst, Ordering::SeqCst, |count| {
                Some(count.saturating_add(intervals).min(MAX_PERFORMANCE_COUNT))
            })
            .expect("clock update always returns a value");
    }
}

impl Clock for MockClock {
    fn now_filetime(&self) -> u64 {
        self.current.load(Ordering::SeqCst)
    }

    fn performance_count(&self) -> u64 {
        self.performance_count.load(Ordering::SeqCst)
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

enum GuestObject {
    Thread(u32),
    Waitable {
        state: Arc<Mutex<events::Waitable>>,
        access: u32,
    },
}

#[derive(Default)]
struct ObjectState {
    priority: i32,
    last_handle: u64,
    handles: BTreeMap<u64, GuestObject>,
}

impl ObjectState {
    fn next_handle(&self) -> Option<u64> {
        self.last_handle
            .checked_add(4)
            .filter(|_| self.handles.len() < MAX_OBJECT_HANDLES)
    }

    fn insert_handle(&mut self, handle: u64, object: GuestObject) {
        self.handles.insert(handle, object);
        self.last_handle = handle;
    }

    fn check_access(&self, handle: u64, required: u32) -> Result<(), u32> {
        if handle == CURRENT_THREAD {
            return Ok(());
        }
        match self.handles.get(&handle) {
            Some(GuestObject::Thread(access)) if access & required != 0 => Ok(()),
            Some(GuestObject::Thread(_)) => Err(ERROR_ACCESS_DENIED),
            _ => Err(ERROR_INVALID_HANDLE),
        }
    }
}

/// State container for runtime services.
pub struct ServicesState {
    pub clock: Arc<dyn Clock>,
    clock_origin: u64,
    pub process_id: u32,
    priority_boost_disabled: AtomicBool,
    pub thread_id: u32,
    objects: Mutex<ObjectState>,
    pub last_error: AtomicU32,
    pub next_alloc_addr: AtomicU64,
    pub graphics: Arc<Mutex<crate::graphics::GraphicsContext>>,
}

impl ServicesState {
    pub fn new(clock: Arc<dyn Clock>) -> Self {
        Self {
            clock_origin: clock.performance_count(),
            clock,
            priority_boost_disabled: AtomicBool::new(false),
            process_id: 0x4100, // Synthetic process ID
            thread_id: 0x4104,  // Synthetic main thread ID
            objects: Mutex::new(ObjectState::default()),
            last_error: AtomicU32::new(0),
            next_alloc_addr: AtomicU64::new(0x0000_0002_0000_0000), // 8GB guest heap
            graphics: Arc::new(Mutex::new(crate::graphics::GraphicsContext::new())),
        }
    }
    fn uptime_millis(&self) -> u64 {
        let ticks = self
            .clock
            .performance_count()
            .saturating_sub(self.clock_origin);
        ticks / (PERFORMANCE_FREQUENCY / 1_000)
    }

    /// The scheduling policy recorded for the synthetic process.
    pub fn priority_boost_disabled(&self) -> bool {
        self.priority_boost_disabled.load(Ordering::SeqCst)
    }

    fn fail(&self, error: u32, return_value: u64) -> Result<u64, ExitReason> {
        self.last_error.store(error, Ordering::SeqCst);
        Ok(return_value)
    }

    fn is_current_process(&self, handle: u64) -> bool {
        if handle == CURRENT_PROCESS {
            return true;
        }
        self.last_error
            .store(ERROR_INVALID_HANDLE, Ordering::SeqCst);
        false
    }
}

/// Register standard Windows core and Xbox memory services into the registry.
pub fn register_standard_services(registry: &mut ImportRegistry, state: Arc<ServicesState>) {
    let s = state.clone();
    registry.register(KERNEL32, "SetProcessPriorityBoost", move |args, _, mem| {
        if !s.is_current_process(args.get(0, mem).map_err(ExitReason::MemoryFault)?) {
            return Ok(0);
        }
        let disable = args.get(1, mem).map_err(ExitReason::MemoryFault)? as u32;
        s.priority_boost_disabled
            .store(disable != 0, Ordering::SeqCst);
        Ok(1)
    });
    registry.alias(PROCESS_THREADS_V2, KERNEL32, "SetProcessPriorityBoost");

    registry.set_status(KERNEL32, "SetProcessPriorityBoost", ServiceStatus::Partial);

    let s = state.clone();
    registry.register(KERNEL32, "GetProcessPriorityBoost", move |args, _, mem| {
        if !s.is_current_process(args.get(0, mem).map_err(ExitReason::MemoryFault)?) {
            return Ok(0);
        }
        let output = args.get(1, mem).map_err(ExitReason::MemoryFault)?;
        if mem
            .write_u32(output, u32::from(s.priority_boost_disabled()))
            .is_err()
        {
            s.last_error.store(ERROR_NOACCESS, Ordering::SeqCst);
            return Ok(0);
        }
        Ok(1)
    });
    registry.alias(PROCESS_THREADS_V2, KERNEL32, "GetProcessPriorityBoost");
    registry.set_status(KERNEL32, "GetProcessPriorityBoost", ServiceStatus::Partial);

    registry.register(KERNEL32, "GetCurrentProcess", |_, _, _| Ok(CURRENT_PROCESS));
    registry.alias(PROCESS_THREADS_V1, KERNEL32, "GetCurrentProcess");

    let s = state.clone();
    registry.register(KERNEL32, "GetProcessId", move |args, _, mem| {
        if !s.is_current_process(args.get(0, mem).map_err(ExitReason::MemoryFault)?) {
            return Ok(0);
        }
        Ok(u64::from(s.process_id))
    });
    registry.alias(PROCESS_THREADS_V1, KERNEL32, "GetProcessId");
    registry.set_status(KERNEL32, "GetProcessId", ServiceStatus::Partial);

    registry.register(KERNEL32, "GetCurrentThread", |_, _, _| Ok(CURRENT_THREAD));
    registry.alias(PROCESS_THREADS_V1, KERNEL32, "GetCurrentThread");

    let s = state.clone();
    registry.register(KERNEL32, "OpenThread", move |args, _, mem| {
        let access = args.get(0, mem).map_err(ExitReason::MemoryFault)? as u32;
        let inherit = args.get(1, mem).map_err(ExitReason::MemoryFault)? as u32;
        let id = args.get(2, mem).map_err(ExitReason::MemoryFault)? as u32;
        if id != s.thread_id {
            return s.fail(ERROR_INVALID_PARAMETER, 0);
        }
        if inherit != 0 || access & !THREAD_SUPPORTED_RIGHTS != 0 {
            return s.fail(ERROR_NOT_SUPPORTED, 0);
        }
        let mut thread = s.objects.lock().expect("guest object state");
        let Some(handle) = thread.next_handle() else {
            return s.fail(ERROR_NOT_ENOUGH_MEMORY, 0);
        };
        thread.insert_handle(handle, GuestObject::Thread(access));
        Ok(handle)
    });

    let s = state.clone();
    registry.register(KERNEL32, "GetThreadPriority", move |args, _, mem| {
        let handle = args.get(0, mem).map_err(ExitReason::MemoryFault)?;
        let thread = s.objects.lock().expect("guest object state");
        if let Err(error) = thread.check_access(handle, THREAD_QUERY_RIGHTS) {
            return s.fail(error, THREAD_PRIORITY_ERROR_RETURN);
        }
        Ok(u64::from(thread.priority as u32))
    });

    let s = state.clone();
    registry.register(KERNEL32, "SetThreadPriority", move |args, _, mem| {
        let handle = args.get(0, mem).map_err(ExitReason::MemoryFault)?;
        let priority = args.get(1, mem).map_err(ExitReason::MemoryFault)? as i32;
        let mut thread = s.objects.lock().expect("guest object state");
        if let Err(error) = thread.check_access(handle, THREAD_SET_RIGHTS) {
            return s.fail(error, 0);
        }
        if matches!(priority, 0x0001_0000 | 0x0002_0000) {
            return s.fail(ERROR_NOT_SUPPORTED, 0);
        }
        if !matches!(priority, -15 | -2..=2 | 15) {
            return s.fail(ERROR_INVALID_PARAMETER, 0);
        }
        thread.priority = priority;
        Ok(1)
    });

    let s = state.clone();
    registry.register(KERNEL32, "GetThreadId", move |args, _, mem| {
        let handle = args.get(0, mem).map_err(ExitReason::MemoryFault)?;
        let thread = s.objects.lock().expect("guest object state");
        if let Err(error) = thread.check_access(handle, THREAD_QUERY_RIGHTS) {
            return s.fail(error, 0);
        }
        Ok(u64::from(s.thread_id))
    });

    for symbol in [
        "OpenThread",
        "GetThreadPriority",
        "SetThreadPriority",
        "GetThreadId",
    ] {
        registry.alias(PROCESS_THREADS_V1, KERNEL32, symbol);
        registry.set_status(KERNEL32, symbol, ServiceStatus::Partial);
    }

    let s = state.clone();
    registry.register(KERNEL32, "CloseHandle", move |args, _, mem| {
        let handle = args.get(0, mem).map_err(ExitReason::MemoryFault)?;
        if matches!(handle, CURRENT_PROCESS | CURRENT_THREAD)
            || s.objects
                .lock()
                .expect("guest object state")
                .handles
                .remove(&handle)
                .is_some()
        {
            return Ok(1);
        }
        s.fail(ERROR_INVALID_HANDLE, 0)
    });
    registry.alias(HANDLE_API, KERNEL32, "CloseHandle");
    registry.set_status(KERNEL32, "CloseHandle", ServiceStatus::Partial);

    let s = state.clone();
    registry.register(KERNEL32, "DuplicateHandle", move |args, _, mem| {
        let source_process = args.get(0, mem).map_err(ExitReason::MemoryFault)?;
        let source = args.get(1, mem).map_err(ExitReason::MemoryFault)?;
        let target_process = args.get(2, mem).map_err(ExitReason::MemoryFault)?;
        let output = args.get(3, mem).map_err(ExitReason::MemoryFault)?;
        let desired = args.get(4, mem).map_err(ExitReason::MemoryFault)? as u32;
        let inherit = args.get(5, mem).map_err(ExitReason::MemoryFault)? as u32;
        let options = args.get(6, mem).map_err(ExitReason::MemoryFault)? as u32;

        if source_process != CURRENT_PROCESS || target_process != CURRENT_PROCESS {
            return s.fail(ERROR_INVALID_HANDLE, 0);
        }
        if inherit != 0 || options & !DUPLICATE_SAME_ACCESS != 0 || source == CURRENT_PROCESS {
            return s.fail(ERROR_NOT_SUPPORTED, 0);
        }
        let mut thread = s.objects.lock().expect("guest object state");
        let (granted, event) = if source == CURRENT_THREAD {
            (THREAD_SUPPORTED_RIGHTS, None)
        } else {
            match thread.handles.get(&source) {
                Some(GuestObject::Thread(rights)) => (*rights, None),
                Some(GuestObject::Waitable { state, access }) => (*access, Some(Arc::clone(state))),
                None => return s.fail(ERROR_INVALID_HANDLE, 0),
            }
        };
        let access = if options & DUPLICATE_SAME_ACCESS != 0 {
            granted
        } else if desired & !granted == 0 {
            desired
        } else {
            return s.fail(ERROR_ACCESS_DENIED, 0);
        };
        let Some(handle) = thread.next_handle() else {
            return s.fail(ERROR_NOT_ENOUGH_MEMORY, 0);
        };
        if output == 0 || mem.write_u64(output, handle).is_err() {
            return s.fail(ERROR_NOACCESS, 0);
        }
        let object = match event {
            Some(state) => GuestObject::Waitable { state, access },
            None => GuestObject::Thread(access),
        };
        thread.insert_handle(handle, object);
        Ok(1)
    });
    registry.alias(HANDLE_API, KERNEL32, "DuplicateHandle");
    registry.set_status(KERNEL32, "DuplicateHandle", ServiceStatus::Partial);

    events::register(registry, &state);
    crt::register(registry);
    crt::register_heap(registry);
    crt::register_errno(registry);
    crt::register_conversions(registry);
    heap::register(registry, &state);

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
    registry.register(KERNEL32, "GetSystemTimeAsFileTime", handler);
    registry.alias(SYSINFO_API, KERNEL32, "GetSystemTimeAsFileTime");

    let s = state.clone();
    registry.register(KERNEL32, "QueryPerformanceCounter", move |args, _, mem| {
        let ptr = args.get(0, mem).map_err(ExitReason::MemoryFault)?;
        if ptr == 0 || mem.write_u64(ptr, s.clock.performance_count()).is_err() {
            return s.fail(ERROR_NOACCESS, 0);
        }
        Ok(1)
    });

    let s = state.clone();
    registry.register(
        KERNEL32,
        "QueryPerformanceFrequency",
        move |args, _, mem| {
            let ptr = args.get(0, mem).map_err(ExitReason::MemoryFault)?;
            if ptr == 0 || mem.write_u64(ptr, PERFORMANCE_FREQUENCY).is_err() {
                return s.fail(ERROR_NOACCESS, 0);
            }
            Ok(1)
        },
    );

    for symbol in ["QueryPerformanceCounter", "QueryPerformanceFrequency"] {
        registry.alias("api-ms-win-core-profile-l1-1-0.dll", KERNEL32, symbol);
    }

    let s = state.clone();
    registry.register(KERNEL32, "GetTickCount", move |_, _, _| {
        Ok(u64::from(s.uptime_millis() as u32))
    });

    let s = state.clone();
    registry.register(KERNEL32, "GetTickCount64", move |_, _, _| {
        Ok(s.uptime_millis())
    });

    for symbol in ["GetTickCount", "GetTickCount64"] {
        registry.alias(SYSINFO_API, KERNEL32, symbol);
    }

    // 2. GetCurrentProcessId() -> DWORD
    let s = state.clone();
    let handler = move |_args: &crate::imports::Win64Args,
                        _cpu: &mut crate::cpu::CpuState,
                        _mem: &mut GuestMemory|
          -> Result<u64, ExitReason> { Ok(s.process_id as u64) };
    registry.register(KERNEL32, "GetCurrentProcessId", handler);
    registry.alias(PROCESS_THREADS_V1, KERNEL32, "GetCurrentProcessId");

    // 3. GetCurrentThreadId() -> DWORD
    let s = state.clone();
    let handler = move |_args: &crate::imports::Win64Args,
                        _cpu: &mut crate::cpu::CpuState,
                        _mem: &mut GuestMemory|
          -> Result<u64, ExitReason> { Ok(s.thread_id as u64) };
    registry.register(KERNEL32, "GetCurrentThreadId", handler);
    registry.alias(PROCESS_THREADS_V1, KERNEL32, "GetCurrentThreadId");

    // 4. GetLastError() -> DWORD
    let s = state.clone();
    let handler =
        move |_args: &crate::imports::Win64Args,
              _cpu: &mut crate::cpu::CpuState,
              _mem: &mut GuestMemory|
              -> Result<u64, ExitReason> { Ok(s.last_error.load(Ordering::SeqCst) as u64) };
    registry.register(KERNEL32, "GetLastError", handler);
    registry.alias(
        "api-ms-win-core-errorhandling-l1-1-0.dll",
        KERNEL32,
        "GetLastError",
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
    registry.register(KERNEL32, "SetLastError", handler);
    registry.alias(
        "api-ms-win-core-errorhandling-l1-1-0.dll",
        KERNEL32,
        "SetLastError",
    );

    let s = state.clone();
    registry.register(KERNEL32, "VirtualAlloc", move |args, _, mem| {
        let requested = args.get(0, mem).map_err(ExitReason::MemoryFault)?;
        let size = args.get(1, mem).map_err(ExitReason::MemoryFault)?;
        let allocation_type = args.get(2, mem).map_err(ExitReason::MemoryFault)? as u32;
        let protect = args.get(3, mem).map_err(ExitReason::MemoryFault)? as u32;

        if size == 0 {
            return s.fail(ERROR_INVALID_PARAMETER, 0);
        }
        if allocation_type != MEM_COMMIT_RESERVE {
            return s.fail(ERROR_NOT_SUPPORTED, 0);
        }
        // Executable allocations require guest translation cache invalidation.
        let permissions = match protect {
            0x01 => MemoryPermissions::NO_ACCESS,
            0x02 => MemoryPermissions::READ_ONLY,
            0x04 => MemoryPermissions::READ_WRITE,
            _ => return s.fail(ERROR_INVALID_PARAMETER, 0),
        };
        let base = requested & !(ALLOCATION_GRANULARITY - 1);
        if requested != 0 && base == 0 {
            return s.fail(ERROR_INVALID_ADDRESS, 0);
        }
        let Some(size) = size
            .checked_add(requested - base)
            .and_then(|span| span.checked_next_multiple_of(PAGE_SIZE))
            .and_then(|span| usize::try_from(span).ok())
        else {
            return s.fail(ERROR_INVALID_PARAMETER, 0);
        };

        match mem.map_virtual_allocation(base, size, permissions) {
            Ok(address) => Ok(address),
            Err(MemoryError::OverlappingRegion { .. } | MemoryError::Overflow) => {
                s.fail(ERROR_INVALID_ADDRESS, 0)
            }
            Err(_) => s.fail(ERROR_NOT_ENOUGH_MEMORY, 0),
        }
    });

    let s = state.clone();
    registry.register(KERNEL32, "VirtualFree", move |args, _, mem| {
        let address = args.get(0, mem).map_err(ExitReason::MemoryFault)?;
        let size = args.get(1, mem).map_err(ExitReason::MemoryFault)?;
        let free_type = args.get(2, mem).map_err(ExitReason::MemoryFault)? as u32;
        if free_type != MEM_RELEASE {
            return s.fail(ERROR_NOT_SUPPORTED, 0);
        }
        if size != 0 {
            return s.fail(ERROR_INVALID_PARAMETER, 0);
        }
        if !mem.release_virtual_allocation(address) {
            return s.fail(ERROR_INVALID_ADDRESS, 0);
        }
        Ok(1)
    });

    for symbol in ["VirtualAlloc", "VirtualFree"] {
        registry.alias("api-ms-win-core-memory-l1-1-0.dll", KERNEL32, symbol);
        registry.set_status(KERNEL32, symbol, ServiceStatus::Partial);
    }

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

        if mem
            .map_region(alloc_addr, aligned_size, MemoryPermissions::READ_WRITE)
            .is_err()
        {
            return Ok(0);
        }

        Ok(alloc_addr)
    };
    registry.register("xmem.dll", "XMemAlloc", handler);
    registry.set_status("xmem.dll", "XMemAlloc", ServiceStatus::Partial);

    // 9. XMemFree(pAddress, dwAllocAttributes) -> VOID (Xbox Scarlett in xmem.dll)
    let handler = move |args: &crate::imports::Win64Args,
                        _cpu: &mut crate::cpu::CpuState,
                        mem: &mut GuestMemory|
          -> Result<u64, ExitReason> {
        let _addr = args.get(0, mem).map_err(ExitReason::MemoryFault)?;
        Ok(0)
    };
    registry.register("xmem.dll", "XMemFree", handler);
    registry.set_status("xmem.dll", "XMemFree", ServiceStatus::Stub);

    // 10. Synthetic Test Graphics Device Creation (xenora_fixture.dll)
    // Prototype graphics device creation for original integration fixtures.
    // In guest ABI: ppDevice is Arg 3 (R9). Writes generation-checked device handle.
    let s_gfx = state.graphics.clone();
    let handler = move |args: &crate::imports::Win64Args,
                        _cpu: &mut crate::cpu::CpuState,
                        mem: &mut GuestMemory|
          -> Result<u64, ExitReason> {
        let pp_device = args.get(3, mem).map_err(ExitReason::MemoryFault)?;
        if pp_device == 0 {
            return Ok(0x80070057); // E_INVALIDARG
        }
        let mut gfx = s_gfx
            .lock()
            .map_err(|_| ExitReason::MemoryFault(crate::memory::MemoryError::Overflow))?;
        match gfx.create_device() {
            Ok(handle) => {
                mem.write_u64(pp_device, handle.to_u64())
                    .map_err(ExitReason::MemoryFault)?;
                Ok(0) // S_OK
            }
            Err(_) => Ok(0x80004005), // E_FAIL
        }
    };
    registry.register("xenora_fixture.dll", "CreateGraphicsDevice", handler);
    registry.set_status(
        "xenora_fixture.dll",
        "CreateGraphicsDevice",
        ServiceStatus::Implemented,
    );

    // 11. Synthetic Test Command Submission & Presentation (xenora_fixture.dll)
    // Prototype graphics submission service:
    // Arg 0 (RCX): Guest address of vertex buffer (12 x f32 coordinates)
    // Arg 1 (RDX): Clear color packed as RGBA u32 or pointer to 4 floats
    // Arg 2 (R8): Viewport dimensions (width in lower 32 bits, height in upper 32 bits)
    // Arg 3 (R9): Optional guest output pointer for readback status or frame index
    let s_gfx = state.graphics.clone();
    let handler = move |args: &crate::imports::Win64Args,
                        _cpu: &mut crate::cpu::CpuState,
                        mem: &mut GuestMemory|
          -> Result<u64, ExitReason> {
        let vtx_addr = args.get(0, mem).map_err(ExitReason::MemoryFault)?;
        if vtx_addr == 0 {
            return Ok(0x80070057); // E_INVALIDARG
        }
        let clear_raw = args.get(1, mem).map_err(ExitReason::MemoryFault)? as u32;
        let clear_color = [
            (clear_raw & 0xFF) as u8,
            ((clear_raw >> 8) & 0xFF) as u8,
            ((clear_raw >> 16) & 0xFF) as u8,
            ((clear_raw >> 24) & 0xFF) as u8,
        ];
        let dims_raw = args.get(2, mem).map_err(ExitReason::MemoryFault)?;
        let width = (dims_raw as u32) as usize;
        let height = ((dims_raw >> 32) as u32) as usize;
        let (vp_w, vp_h) = if width > 0 && height > 0 {
            (width, height)
        } else {
            (4, 3)
        };

        let mut gfx = s_gfx
            .lock()
            .map_err(|_| ExitReason::MemoryFault(crate::memory::MemoryError::Overflow))?;

        // Build and execute guest command list through GraphicsContext
        let dev = gfx
            .create_device()
            .map_err(|_| ExitReason::MemoryFault(crate::memory::MemoryError::Overflow))?;
        let queue = gfx
            .create_command_queue(dev, crate::graphics::CommandListType::Direct)
            .map_err(|_| ExitReason::MemoryFault(crate::memory::MemoryError::Overflow))?;
        let alloc = gfx
            .create_command_allocator(dev, crate::graphics::CommandListType::Direct)
            .map_err(|_| ExitReason::MemoryFault(crate::memory::MemoryError::Overflow))?;
        let list = gfx
            .create_command_list(dev, alloc, crate::graphics::CommandListType::Direct)
            .map_err(|_| ExitReason::MemoryFault(crate::memory::MemoryError::Overflow))?;

        let vb_desc = crate::graphics::ResourceDesc {
            dimension: crate::graphics::ResourceDimension::Buffer,
            width: 48,
            height: 1,
            depth_or_array_size: 1,
            mip_levels: 1,
            format_rgba8: false,
        };
        let vb = gfx
            .create_committed_resource(
                dev,
                vb_desc,
                crate::graphics::ResourceStates::VERTEX_AND_CONSTANT_BUFFER,
                Some(vtx_addr),
            )
            .map_err(|_| ExitReason::MemoryFault(crate::memory::MemoryError::Overflow))?;

        gfx.cmd_set_viewport(list, vp_w, vp_h)
            .map_err(|_| ExitReason::MemoryFault(crate::memory::MemoryError::Overflow))?;
        gfx.cmd_set_render_targets(list, None, clear_color)
            .map_err(|_| ExitReason::MemoryFault(crate::memory::MemoryError::Overflow))?;
        gfx.cmd_set_vertex_buffer(list, vb, 0, 48)
            .map_err(|_| ExitReason::MemoryFault(crate::memory::MemoryError::Overflow))?;
        gfx.cmd_draw_instanced(list, 3, 1, 0, 0)
            .map_err(|_| ExitReason::MemoryFault(crate::memory::MemoryError::Overflow))?;
        gfx.close_command_list(list)
            .map_err(|_| ExitReason::MemoryFault(crate::memory::MemoryError::Overflow))?;

        let outcome = gfx.execute_command_list(queue, list, mem).map_err(|e| {
            ExitReason::IllegalInstruction(match e {
                crate::graphics::GraphicsError::NativeMetalRequired => 0xFE,
                _ => 0xFF,
            })
        })?;

        let pp_out = args.get(3, mem).map_err(ExitReason::MemoryFault)?;
        if pp_out != 0 {
            mem.write_u64(pp_out, outcome.pixels.len() as u64)
                .map_err(ExitReason::MemoryFault)?;
        }

        Ok(0) // S_OK
    };
    registry.register("xenora_fixture.dll", "SubmitGuestDraw", handler);
    registry.set_status(
        "xenora_fixture.dll",
        "SubmitGuestDraw",
        ServiceStatus::Implemented,
    );
}
