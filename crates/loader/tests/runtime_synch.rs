// SPDX-License-Identifier: GPL-3.0-or-later

//! Mutexes, critical sections, SRW locks, condition variables, one-time
//! initialization, SLists, thread-local storage and the CRT services that call
//! guest functions, called by translated guest code through a bound import
//! table.
//!
//! Two guest threads are modelled by switching the TEB address in GS between
//! calls. Guest comparators and initializers are original x86-64 routines in
//! the image's text section. Expected values come from the public Win32
//! synchapi and processthreadsapi documentation, the C standard, the Microsoft
//! CRT reference and the x64 layouts in winnt.h, not from the handlers.

use std::sync::Arc;
use xenora_loader::cpu::{CpuState, ExitReason, Register};
use xenora_loader::dbt::DbtEngine;
use xenora_loader::imports::ImportRegistry;
use xenora_loader::memory::{GuestMemory, MemoryPermissions};
use xenora_loader::pe::ParsedPe;
use xenora_loader::services::{MockClock, ServicesState, register_standard_services};

const BASE: u64 = 0x1_4000_0000;
const CODE_RVA: u32 = 0x1000;
const IDATA_RVA: u32 = 0x10000;
const IDATA_RAW: usize = 0x2200;
const IDATA_LEN: usize = 0x4000;
const SCRATCH: u64 = 0x5_0000;
const STACK: u64 = 0x8_0000;
const STACK_TOP: u64 = STACK + 0x800;
const THREAD_A: u64 = 0x7_0000;
const THREAD_B: u64 = 0x7_2000;

const KERNEL32: &str = "kernel32.dll";
const SYNCH: &str = "api-ms-win-core-synch-l1-1-0.dll";
const SYNCH_2: &str = "api-ms-win-core-synch-l1-2-0.dll";
const SLIST: &str = "api-ms-win-core-interlocked-l1-1-0.dll";
const THREADS: &str = "api-ms-win-core-processthreads-l1-1-0.dll";
const FIBERS: &str = "api-ms-win-core-fibers-l1-1-0.dll";

const IMPORTS: &[(&str, &str)] = &[
    (KERNEL32, "GetLastError"),
    (KERNEL32, "SetLastError"),
    (KERNEL32, "GetCurrentThreadId"),
    (KERNEL32, "CloseHandle"),
    (KERNEL32, "DuplicateHandle"),
    (KERNEL32, "SetEvent"),
    (KERNEL32, "CreateEventW"),
    (KERNEL32, "WaitForSingleObject"),
    (KERNEL32, "WaitForMultipleObjects"),
    (KERNEL32, "CreateMutexW"),
    (SYNCH, "CreateMutexExW"),
    (SYNCH, "ReleaseMutex"),
    (KERNEL32, "InitializeCriticalSection"),
    (KERNEL32, "InitializeCriticalSectionAndSpinCount"),
    (SYNCH, "InitializeCriticalSectionEx"),
    (KERNEL32, "EnterCriticalSection"),
    (SYNCH, "TryEnterCriticalSection"),
    (KERNEL32, "LeaveCriticalSection"),
    (KERNEL32, "DeleteCriticalSection"),
    (KERNEL32, "SetCriticalSectionSpinCount"),
    (SYNCH, "InitializeSRWLock"),
    (KERNEL32, "AcquireSRWLockExclusive"),
    (SYNCH, "AcquireSRWLockShared"),
    (KERNEL32, "TryAcquireSRWLockExclusive"),
    (KERNEL32, "TryAcquireSRWLockShared"),
    (KERNEL32, "ReleaseSRWLockExclusive"),
    (SYNCH, "ReleaseSRWLockShared"),
    (KERNEL32, "InitializeConditionVariable"),
    (KERNEL32, "WakeAllConditionVariable"),
    (SYNCH_2, "WakeConditionVariable"),
    (KERNEL32, "SleepConditionVariableCS"),
    (SYNCH_2, "SleepConditionVariableSRW"),
    (SYNCH_2, "InitOnceInitialize"),
    (KERNEL32, "InitOnceBeginInitialize"),
    (SYNCH_2, "InitOnceComplete"),
    (KERNEL32, "InitializeSListHead"),
    (SLIST, "InterlockedPushEntrySList"),
    (KERNEL32, "InterlockedPopEntrySList"),
    (SLIST, "InterlockedFlushSList"),
    (SLIST, "QueryDepthSList"),
    (KERNEL32, "TlsAlloc"),
    (THREADS, "TlsGetValue"),
    (THREADS, "TlsSetValue"),
    (KERNEL32, "TlsFree"),
    (FIBERS, "FlsAlloc"),
    (KERNEL32, "FlsGetValue"),
    (FIBERS, "FlsSetValue"),
    (FIBERS, "FlsFree"),
    (SYNCH_2, "InitOnceExecuteOnce"),
    (UCRT, "qsort"),
    (CRT_UTILITY, "qsort_s"),
    (CRT_UTILITY, "bsearch"),
    (UCRT, "bsearch_s"),
    (CRT_RUNTIME, "_initterm"),
    (UCRT, "_initterm_e"),
];

const UCRT: &str = "ucrtbase.dll";
const CRT_UTILITY: &str = "api-ms-win-crt-utility-l1-1-0.dll";
const CRT_RUNTIME: &str = "api-ms-win-crt-runtime-l1-1-0.dll";

/// Guest routines, placed at RVA 0x1800 in the text section.
const ROUTINES_RVA: u32 = 0x1800;
/// Written by the initializer and cleanup routines.
const LOG: u64 = SCRATCH + 0xF00;
const COUNTER: u64 = SCRATCH + 0xF08;

#[derive(Clone, Copy)]
enum Routine {
    /// `int compare(const int *a, const int *b)`: `*a - *b`.
    Compare,
    /// `int compare_s(int64_t *calls, const int *a, const int *b)`: counts calls.
    CompareCounting,
    /// `void init(void)` appending `n` as a decimal digit of LOG.
    Append(u8),
    /// `int init(void)` returning 7 without touching LOG.
    Fail,
    /// Clobbers nonvolatile RBX.
    ClobberRbx,
    /// `BOOL init(INIT_ONCE *, void *parameter, void **context)`: stores the
    /// parameter as the context, increments COUNTER and returns TRUE.
    InitOnceStore,
    /// `BOOL init(...)` returning FALSE.
    InitOnceFail,
    /// `void cleanup(void *value)` adding the value to LOG.
    FlsAdd,
}

const ROUTINES: [Routine; 10] = [
    Routine::Compare,
    Routine::CompareCounting,
    Routine::Append(1),
    Routine::Append(2),
    Routine::Append(3),
    Routine::Fail,
    Routine::ClobberRbx,
    Routine::InitOnceStore,
    Routine::InitOnceFail,
    Routine::FlsAdd,
];

impl Routine {
    fn code(self) -> Vec<u8> {
        let mov_rax = |address: u64| {
            let mut code = vec![0x48, 0xB8]; // mov rax, imm64
            code.extend_from_slice(&address.to_le_bytes());
            code
        };
        match self {
            // mov eax, [rcx]; sub eax, [rdx]; ret
            Self::Compare => vec![0x8B, 0x01, 0x2B, 0x02, 0xC3],
            // inc qword [rcx]; mov eax, [rdx]; sub eax, [r8]; ret
            Self::CompareCounting => vec![0x48, 0xFF, 0x01, 0x8B, 0x02, 0x41, 0x2B, 0x00, 0xC3],
            Self::Append(digit) => {
                let mut code = mov_rax(LOG);
                code.extend_from_slice(&[
                    0x48, 0x8B, 0x08, // mov rcx, [rax]
                    0x48, 0x6B, 0xC9, 0x0A, // imul rcx, rcx, 10
                    0x48, 0x83, 0xC1, digit, // add rcx, digit
                    0x48, 0x89, 0x08, // mov [rax], rcx
                    0x31, 0xC0, // xor eax, eax
                    0xC3,
                ]);
                code
            }
            // mov eax, 7; ret
            Self::Fail => vec![0xB8, 7, 0, 0, 0, 0xC3],
            // mov rbx, 1; ret
            Self::ClobberRbx => vec![0x48, 0xC7, 0xC3, 1, 0, 0, 0, 0xC3],
            Self::InitOnceStore => {
                let mut code = mov_rax(COUNTER);
                code.extend_from_slice(&[
                    0x48, 0xFF, 0x00, // inc qword [rax]
                    0x49, 0x89, 0x10, // mov [r8], rdx
                    0xB8, 1, 0, 0, 0, // mov eax, 1
                    0xC3,
                ]);
                code
            }
            // xor eax, eax; ret
            Self::InitOnceFail => vec![0x31, 0xC0, 0xC3],
            Self::FlsAdd => {
                let mut code = mov_rax(LOG);
                code.extend_from_slice(&[0x48, 0x01, 0x08, 0xC3]); // add [rax], rcx; ret
                code
            }
        }
    }
}

/// Guest address of a routine. Each routine occupies a 32-byte slot.
fn routine(which: usize) -> u64 {
    BASE + u64::from(ROUTINES_RVA) + 32 * which as u64
}

const COMPARE: usize = 0;
const COMPARE_COUNTING: usize = 1;
const APPEND_1: usize = 2;
const APPEND_2: usize = 3;
const APPEND_3: usize = 4;
const FAIL: usize = 5;
const CLOBBER_RBX: usize = 6;
const INIT_ONCE_STORE: usize = 7;
const INIT_ONCE_FAIL: usize = 8;
const FLS_ADD: usize = 9;

const WAIT_TIMEOUT: u64 = 258;
const ERROR_INVALID_HANDLE: u64 = 6;
const ERROR_GEN_FAILURE: u64 = 31;
const ERROR_NOT_SUPPORTED: u64 = 50;
const ERROR_INVALID_PARAMETER: u64 = 87;
const ERROR_NOT_OWNER: u64 = 288;
const ERROR_TIMEOUT: u64 = 1460;
const RUNTIME_LIMIT: ExitReason = ExitReason::IllegalInstruction(0xFE);

/// Original PE32+ image importing each entry of `IMPORTS` from its own
/// descriptor. The code for import `n`, at RVA 0x1000 + 16n, is
/// `call [its slot]; hlt`.
fn build() -> Vec<u8> {
    let mut pe = vec![0u8; IDATA_RAW + IDATA_LEN];
    fn put(pe: &mut [u8], at: usize, bytes: &[u8]) {
        pe[at..at + bytes.len()].copy_from_slice(bytes);
    }
    put(&mut pe, 0, b"MZ");
    put(&mut pe, 0x3C, &0x80u32.to_le_bytes());
    put(&mut pe, 0x80, b"PE\0\0");
    put(&mut pe, 0x84, &0x8664u16.to_le_bytes());
    put(&mut pe, 0x86, &2u16.to_le_bytes());
    put(&mut pe, 0x94, &240u16.to_le_bytes());
    let optional = 0x98;
    put(&mut pe, optional, &0x20Bu16.to_le_bytes());
    put(&mut pe, optional + 16, &CODE_RVA.to_le_bytes());
    put(&mut pe, optional + 24, &BASE.to_le_bytes());
    put(&mut pe, optional + 32, &0x1000u32.to_le_bytes());
    put(&mut pe, optional + 36, &0x200u32.to_le_bytes());
    put(&mut pe, optional + 56, &0x20000u32.to_le_bytes());
    put(&mut pe, optional + 60, &0x200u32.to_le_bytes());
    put(&mut pe, optional + 108, &16u32.to_le_bytes());
    put(&mut pe, optional + 120, &IDATA_RVA.to_le_bytes());
    put(&mut pe, optional + 124, &0x100u32.to_le_bytes());
    for (index, (name, rva, raw, len, flags)) in [
        (b".text", CODE_RVA, 0x200, 0x2000, 0x6000_0020u32),
        (b".idat", IDATA_RVA, IDATA_RAW, IDATA_LEN, 0xC000_0040),
    ]
    .into_iter()
    .enumerate()
    {
        let section = optional + 240 + 40 * index;
        put(&mut pe, section, name);
        put(&mut pe, section + 8, &(len as u32).to_le_bytes());
        put(&mut pe, section + 12, &rva.to_le_bytes());
        put(&mut pe, section + 16, &(len as u32).to_le_bytes());
        put(&mut pe, section + 20, &(raw as u32).to_le_bytes());
        put(&mut pe, section + 36, &flags.to_le_bytes());
    }

    let file = |rva: u32| IDATA_RAW + (rva - IDATA_RVA) as usize;
    let mut next = IDATA_RVA + 20 * (IMPORTS.len() as u32 + 1);
    let mut take = |len: usize| {
        let rva = next;
        next = (next + len as u32 + 7) & !7;
        rva
    };
    for (index, (module, symbol)) in IMPORTS.iter().enumerate() {
        let module_rva = take(module.len() + 1);
        put(&mut pe, file(module_rva), module.as_bytes());
        let hint = take(symbol.len() + 3);
        put(&mut pe, file(hint) + 2, symbol.as_bytes());
        let (lookup, slot) = (take(16), take(16));
        put(&mut pe, file(lookup), &u64::from(hint).to_le_bytes());
        put(&mut pe, file(slot), &u64::from(hint).to_le_bytes());
        let descriptor = file(IDATA_RVA) + 20 * index;
        put(&mut pe, descriptor, &lookup.to_le_bytes());
        put(&mut pe, descriptor + 12, &module_rva.to_le_bytes());
        put(&mut pe, descriptor + 16, &slot.to_le_bytes());

        let code_rva = CODE_RVA + 16 * index as u32;
        let displacement = slot.wrapping_sub(code_rva + 6);
        let at = 0x200 + 16 * index;
        put(&mut pe, at, &[0xFF, 0x15]); // call qword ptr [rip + displacement]
        put(&mut pe, at + 2, &displacement.to_le_bytes());
        put(&mut pe, at + 6, &[0xF4]); // hlt
    }
    for (index, routine) in ROUTINES.iter().enumerate() {
        let code = routine.code();
        assert!(code.len() <= 32);
        let at = 0x200 + (ROUTINES_RVA - CODE_RVA) as usize + 32 * index;
        put(&mut pe, at, &code);
    }
    pe
}

struct Guest {
    engine: DbtEngine,
    memory: GuestMemory,
    teb: u64,
}

impl Guest {
    fn new() -> Self {
        let bytes = build();
        let pe = ParsedPe::parse(&bytes).unwrap();
        let mut memory = pe.load_into_memory(&bytes).unwrap().memory;
        memory
            .map_region(SCRATCH, 0x1000, MemoryPermissions::READ_WRITE)
            .unwrap();
        memory
            .map_region(STACK, 0x1000, MemoryPermissions::READ_WRITE)
            .unwrap();
        let mut registry = ImportRegistry::new();
        let state = ServicesState::new(Arc::new(MockClock::new(0)));
        register_standard_services(&mut registry, Arc::new(state));
        let bound = registry
            .bind_imports(&pe, &bytes, BASE, &mut memory)
            .unwrap();
        assert_eq!(bound, IMPORTS.len());
        let mut engine = DbtEngine::new(256 * 1024).unwrap();
        engine.set_import_registry(registry);
        Self {
            engine,
            memory,
            teb: THREAD_A,
        }
    }

    fn on(&mut self, teb: u64) -> &mut Self {
        self.teb = teb;
        self
    }

    fn run(&mut self, symbol: &str, args: &[u64]) -> (u64, ExitReason) {
        let index = IMPORTS
            .iter()
            .position(|(_, name)| *name == symbol)
            .unwrap_or_else(|| panic!("{symbol} is not imported"));
        let mut cpu = CpuState::new(BASE + u64::from(CODE_RVA) + 16 * index as u64, STACK_TOP);
        cpu.gs_base = self.teb;
        for (register, value) in [Register::Rcx, Register::Rdx, Register::R8, Register::R9]
            .into_iter()
            .zip(args)
        {
            cpu.set(register, *value);
        }
        // Stack arguments start after the return address and the home area.
        for (slot, value) in args.iter().skip(4).enumerate() {
            let address = STACK_TOP - 8 + 8 + 32 + 8 * slot as u64;
            self.memory.write_u64(address, *value).unwrap();
        }
        use Register::{R12, R13, R14, R15, Rbp, Rbx, Rdi, Rsi};
        let nonvolatile = [Rbx, Rbp, Rdi, Rsi, R12, R13, R14, R15];
        for (index, register) in nonvolatile.into_iter().enumerate() {
            cpu.set(register, 0x5EED_0000 + index as u64);
        }
        let result = self.engine.run(&mut cpu, &mut self.memory, 100_000);
        if result.exit_reason == ExitReason::Halt {
            // The service returned to the instruction after its CALL.
            assert_eq!(cpu.get(Register::Rsp), STACK_TOP, "{symbol}");
            for (index, register) in nonvolatile.into_iter().enumerate() {
                assert_eq!(cpu.get(register), 0x5EED_0000 + index as u64, "{symbol}");
            }
        }
        (cpu.get(Register::Rax), result.exit_reason)
    }

    fn call(&mut self, symbol: &str, args: &[u64]) -> u64 {
        let (rax, exit) = self.run(symbol, args);
        assert_eq!(exit, ExitReason::Halt, "{symbol}");
        rax
    }

    fn fails(&mut self, symbol: &str, args: &[u64]) -> ExitReason {
        let (_, exit) = self.run(symbol, args);
        assert_ne!(exit, ExitReason::Halt, "{symbol} should stop the guest");
        exit
    }

    fn error(&mut self) -> u64 {
        self.call("GetLastError", &[])
    }

    fn u32_at(&self, address: u64) -> u32 {
        self.memory.read_u32(address).unwrap()
    }

    fn u64_at(&self, address: u64) -> u64 {
        self.memory.read_u64(address).unwrap()
    }
}

#[test]
fn mutexes_are_recursive_for_their_owner_and_exclusive_across_threads() {
    let mut guest = Guest::new();
    let mutex = guest.on(THREAD_A).call("CreateMutexW", &[0, 1, 0]);
    assert_ne!(mutex, 0);

    // A held mutex is not signaled for another thread, but its owner may
    // acquire it again; each acquisition needs a matching release.
    assert_eq!(
        guest.on(THREAD_B).call("WaitForSingleObject", &[mutex, 0]),
        WAIT_TIMEOUT
    );
    assert_eq!(
        guest.on(THREAD_A).call("WaitForSingleObject", &[mutex, 0]),
        0
    );
    assert_eq!(guest.on(THREAD_B).call("ReleaseMutex", &[mutex]), 0);
    assert_eq!(guest.error(), ERROR_NOT_OWNER);
    assert_eq!(guest.on(THREAD_A).call("ReleaseMutex", &[mutex]), 1);
    assert_eq!(
        guest.on(THREAD_B).call("WaitForSingleObject", &[mutex, 0]),
        WAIT_TIMEOUT
    );
    assert_eq!(guest.on(THREAD_A).call("ReleaseMutex", &[mutex]), 1);
    assert_eq!(guest.call("ReleaseMutex", &[mutex]), 0);
    assert_eq!(guest.error(), ERROR_NOT_OWNER);

    // Once released, the next waiter acquires ownership.
    assert_eq!(
        guest.on(THREAD_B).call("WaitForSingleObject", &[mutex, 0]),
        0
    );
    assert_eq!(
        guest.on(THREAD_A).call("WaitForSingleObject", &[mutex, 0]),
        WAIT_TIMEOUT
    );
    // A wait that would block needs the guest scheduler.
    assert_eq!(
        guest.call("WaitForSingleObject", &[mutex, 10]),
        u64::from(u32::MAX)
    );
    assert_eq!(guest.error(), ERROR_NOT_SUPPORTED);

    // A duplicate refers to the same mutex, so it shares ownership.
    let output = SCRATCH + 0x800;
    let duplicate = guest.on(THREAD_B).call(
        "DuplicateHandle",
        &[u64::MAX, mutex, u64::MAX, output, 0, 0, 2],
    );
    assert_eq!(duplicate, 1);
    let duplicate = guest.u64_at(output);
    assert_eq!(guest.call("CloseHandle", &[mutex]), 1);
    assert_eq!(
        guest
            .on(THREAD_A)
            .call("WaitForSingleObject", &[duplicate, 0]),
        WAIT_TIMEOUT
    );
    assert_eq!(guest.on(THREAD_B).call("ReleaseMutex", &[duplicate]), 1);
    assert_eq!(
        guest
            .on(THREAD_A)
            .call("WaitForSingleObject", &[duplicate, 0]),
        0
    );

    // Mutexes are not events.
    assert_eq!(guest.call("SetEvent", &[duplicate]), 0);
    assert_eq!(guest.error(), ERROR_INVALID_HANDLE);

    // CREATE_MUTEX_INITIAL_OWNER is the only defined flag.
    let owned = guest.call("CreateMutexExW", &[0, 0, 1, 0x001f_0001]);
    assert_ne!(owned, 0);
    assert_eq!(
        guest.on(THREAD_B).call("WaitForSingleObject", &[owned, 0]),
        WAIT_TIMEOUT
    );
    assert_eq!(guest.call("CreateMutexExW", &[0, 0, 2, 0x001f_0001]), 0);
    assert_eq!(guest.error(), ERROR_INVALID_PARAMETER);

    // Wait-all acquires the mutex and consumes the event together, or neither.
    let event = guest.call("CreateEventW", &[0, 0, 1, 0]);
    let unowned = guest.call("CreateMutexW", &[0, 0, 0]);
    let handles = SCRATCH + 0x900;
    for (index, handle) in [unowned, event, owned].into_iter().enumerate() {
        guest
            .memory
            .write_u64(handles + 8 * index as u64, handle)
            .unwrap();
    }
    assert_eq!(
        guest
            .on(THREAD_B)
            .call("WaitForMultipleObjects", &[3, handles, 1, 0]),
        WAIT_TIMEOUT
    );
    assert_eq!(guest.call("WaitForSingleObject", &[event, 0]), 0);
    assert_eq!(guest.call("SetEvent", &[event]), 1);
    assert_eq!(
        guest
            .on(THREAD_A)
            .call("WaitForMultipleObjects", &[3, handles, 1, 0]),
        0
    );
    assert_eq!(
        guest
            .on(THREAD_B)
            .call("WaitForSingleObject", &[unowned, 0]),
        WAIT_TIMEOUT
    );
    assert_eq!(guest.call("WaitForSingleObject", &[event, 0]), WAIT_TIMEOUT);
}

#[test]
fn critical_sections_track_owner_and_recursion_in_the_guest_structure() {
    let mut guest = Guest::new();
    let section = SCRATCH;
    guest.memory.write_bytes(section, &[0xAA; 48]).unwrap();
    assert_eq!(guest.call("InitializeCriticalSection", &[section]), 0);
    // winnt.h RTL_CRITICAL_SECTION: LockCount at 8 is -1 when free,
    // RecursionCount at 12, OwningThread at 16, SpinCount at 32.
    assert_eq!(guest.u32_at(section + 8), u32::MAX);
    assert_eq!(guest.u32_at(section + 12), 0);
    assert_eq!(guest.u64_at(section + 16), 0);
    assert_eq!(guest.u64_at(section + 32), 0);
    assert_eq!(guest.u64_at(section + 40), 0xAAAA_AAAA_AAAA_AAAA);

    let thread_id = guest.on(THREAD_A).call("GetCurrentThreadId", &[]);
    assert_eq!(guest.call("EnterCriticalSection", &[section]), 0);
    assert_eq!(guest.call("TryEnterCriticalSection", &[section]), 1);
    assert_eq!(guest.u32_at(section + 12), 2);
    assert_eq!(guest.u64_at(section + 16), thread_id);
    assert_ne!(guest.u32_at(section + 8), u32::MAX);

    assert_eq!(
        guest
            .on(THREAD_B)
            .call("TryEnterCriticalSection", &[section]),
        0
    );
    assert_eq!(
        guest.fails("EnterCriticalSection", &[section]),
        RUNTIME_LIMIT
    );
    assert_eq!(
        guest.fails("LeaveCriticalSection", &[section]),
        RUNTIME_LIMIT
    );
    assert_eq!(guest.u32_at(section + 12), 2);

    assert_eq!(
        guest.on(THREAD_A).call("LeaveCriticalSection", &[section]),
        0
    );
    assert_eq!(guest.u32_at(section + 12), 1);
    assert_eq!(guest.call("LeaveCriticalSection", &[section]), 0);
    assert_eq!(guest.u32_at(section + 8), u32::MAX);
    assert_eq!(guest.u64_at(section + 16), 0);
    assert_eq!(
        guest
            .on(THREAD_B)
            .call("TryEnterCriticalSection", &[section]),
        1
    );
    assert_eq!(guest.call("LeaveCriticalSection", &[section]), 0);
    assert_eq!(guest.call("DeleteCriticalSection", &[section]), 0);
    assert_eq!(
        guest.fails("EnterCriticalSection", &[section]),
        RUNTIME_LIMIT
    );

    let spinning = SCRATCH + 0x40;
    assert_eq!(
        guest.call("InitializeCriticalSectionAndSpinCount", &[spinning, 4000]),
        1
    );
    assert_eq!(guest.u64_at(spinning + 32), 4000);
    assert_eq!(
        guest.call("SetCriticalSectionSpinCount", &[spinning, 100]),
        4000
    );
    assert_eq!(guest.u64_at(spinning + 32), 100);

    let extended = SCRATCH + 0x80;
    // CRITICAL_SECTION_NO_DEBUG_INFO is accepted; other flag bits are reserved.
    assert_eq!(
        guest.call("InitializeCriticalSectionEx", &[extended, 0, 0x0100_0000]),
        1
    );
    assert_eq!(
        guest.call(
            "InitializeCriticalSectionEx",
            &[extended + 0x40, 0, 0x0200_0000]
        ),
        0
    );
    assert_eq!(guest.error(), ERROR_INVALID_PARAMETER);

    // The structure lives in guest memory, so it must be writable.
    let text = BASE + u64::from(CODE_RVA);
    assert!(matches!(
        guest.fails("InitializeCriticalSection", &[text]),
        ExitReason::MemoryFault(_)
    ));
}

#[test]
fn srw_locks_allow_shared_readers_or_one_writer_and_condition_sleeps_time_out() {
    let mut guest = Guest::new();
    let lock = SCRATCH;
    let condition = SCRATCH + 8;
    let section = SCRATCH + 0x40;
    guest.memory.write_u64(lock, u64::MAX).unwrap();
    assert_eq!(guest.call("InitializeSRWLock", &[lock]), 0);
    assert_eq!(guest.u64_at(lock), 0); // SRWLOCK_INIT

    assert_eq!(guest.on(THREAD_A).call("AcquireSRWLockShared", &[lock]), 0);
    assert_eq!(
        guest.on(THREAD_B).call("TryAcquireSRWLockShared", &[lock]) & 0xff,
        1
    );
    assert_eq!(guest.call("TryAcquireSRWLockExclusive", &[lock]) & 0xff, 0);
    assert_eq!(
        guest.fails("ReleaseSRWLockExclusive", &[lock]),
        RUNTIME_LIMIT
    );
    assert_eq!(guest.call("ReleaseSRWLockShared", &[lock]), 0);
    assert_eq!(guest.call("TryAcquireSRWLockExclusive", &[lock]) & 0xff, 0);
    assert_eq!(guest.on(THREAD_A).call("ReleaseSRWLockShared", &[lock]), 0);

    assert_eq!(
        guest
            .on(THREAD_B)
            .call("TryAcquireSRWLockExclusive", &[lock])
            & 0xff,
        1
    );
    assert_eq!(
        guest.on(THREAD_A).call("TryAcquireSRWLockShared", &[lock]) & 0xff,
        0
    );
    assert_eq!(
        guest.fails("AcquireSRWLockExclusive", &[lock]),
        RUNTIME_LIMIT
    );
    assert_eq!(
        guest.fails("ReleaseSRWLockExclusive", &[lock]),
        RUNTIME_LIMIT
    );
    // Exclusive SRW locks are not recursive.
    assert_eq!(
        guest.on(THREAD_B).fails("AcquireSRWLockExclusive", &[lock]),
        RUNTIME_LIMIT
    );

    guest.memory.write_u64(condition, u64::MAX).unwrap();
    assert_eq!(guest.call("InitializeConditionVariable", &[condition]), 0);
    assert_eq!(guest.u64_at(condition), 0); // CONDITION_VARIABLE_INIT
    assert_eq!(guest.call("WakeConditionVariable", &[condition]), 0);
    assert_eq!(guest.call("WakeAllConditionVariable", &[condition]), 0);

    // A zero-timeout sleep returns FALSE with ERROR_TIMEOUT and still holds
    // the lock in its original mode.
    assert_eq!(
        guest.call("SleepConditionVariableSRW", &[condition, lock, 0, 0]),
        0
    );
    assert_eq!(guest.error(), ERROR_TIMEOUT);
    assert_eq!(
        guest.on(THREAD_A).call("TryAcquireSRWLockShared", &[lock]) & 0xff,
        0
    );
    assert_eq!(
        guest
            .on(THREAD_B)
            .call("SleepConditionVariableSRW", &[condition, lock, 10, 0]),
        0
    );
    assert_eq!(guest.error(), ERROR_NOT_SUPPORTED);
    assert_eq!(
        guest.call("SleepConditionVariableSRW", &[condition, lock, 0, 2]),
        0
    );
    assert_eq!(guest.error(), ERROR_INVALID_PARAMETER);
    // Shared mode does not match the exclusive hold; the caller must own the lock.
    assert_eq!(
        guest.fails("SleepConditionVariableSRW", &[condition, lock, 0, 1]),
        RUNTIME_LIMIT
    );
    assert_eq!(guest.call("ReleaseSRWLockExclusive", &[lock]), 0);
    assert_eq!(
        guest.fails("SleepConditionVariableSRW", &[condition, lock, 0, 0]),
        RUNTIME_LIMIT
    );

    assert_eq!(guest.call("InitializeCriticalSection", &[section]), 0);
    assert_eq!(
        guest.fails("SleepConditionVariableCS", &[condition, section, 0]),
        RUNTIME_LIMIT
    );
    assert_eq!(guest.call("EnterCriticalSection", &[section]), 0);
    assert_eq!(guest.call("EnterCriticalSection", &[section]), 0);
    assert_eq!(
        guest.call("SleepConditionVariableCS", &[condition, section, 0]),
        0
    );
    assert_eq!(guest.error(), ERROR_TIMEOUT);
    assert_eq!(guest.u32_at(section + 12), 2);
    assert_eq!(
        guest
            .on(THREAD_A)
            .call("TryEnterCriticalSection", &[section]),
        0
    );
}

#[test]
fn init_once_runs_one_initializer_and_publishes_its_context() {
    let mut guest = Guest::new();
    let once = SCRATCH;
    let pending = SCRATCH + 0x10;
    let context = SCRATCH + 0x18;
    let begin = |guest: &mut Guest, flags: u64| {
        guest.memory.write_u64(pending, u64::MAX).unwrap();
        guest.memory.write_u64(context, u64::MAX).unwrap();
        guest.call("InitOnceBeginInitialize", &[once, flags, pending, context])
    };
    guest.memory.write_u64(once, u64::MAX).unwrap();
    assert_eq!(guest.call("InitOnceInitialize", &[once]), 0);
    assert_eq!(guest.u64_at(once), 0); // INIT_ONCE_STATIC_INIT

    // INIT_ONCE_CHECK_ONLY reports incomplete initialization as a failure.
    assert_eq!(begin(&mut guest, 1), 0);
    assert_eq!(guest.error(), ERROR_GEN_FAILURE);
    assert_eq!(guest.u64_at(pending), u64::MAX);

    assert_eq!(begin(&mut guest, 0), 1);
    // fPending is a 4-byte BOOL.
    assert_eq!(guest.u64_at(pending), 0xffff_ffff_0000_0001);
    assert_eq!(begin(&mut guest, 1), 0);

    // Context values must leave INIT_ONCE_CTX_RESERVED_BITS (2) clear.
    assert_eq!(guest.call("InitOnceComplete", &[once, 0, 0x1003]), 0);
    assert_eq!(guest.error(), ERROR_INVALID_PARAMETER);
    // Only the thread that began initialization may complete it.
    assert_eq!(
        guest
            .on(THREAD_B)
            .call("InitOnceComplete", &[once, 0, 0x1000]),
        0
    );
    assert_eq!(guest.error(), ERROR_GEN_FAILURE);
    assert_eq!(
        guest
            .on(THREAD_A)
            .call("InitOnceComplete", &[once, 0, 0x1000]),
        1
    );

    for flags in [0, 1] {
        assert_eq!(begin(guest.on(THREAD_B), flags), 1);
        assert_eq!(guest.u64_at(pending), 0xffff_ffff_0000_0000);
        assert_eq!(guest.u64_at(context), 0x1000);
    }
    assert_eq!(guest.call("InitOnceComplete", &[once, 0, 0x1000]), 0);
    assert_eq!(guest.error(), ERROR_GEN_FAILURE);

    // INIT_ONCE_INIT_FAILED returns the object to its initial state.
    let retry = SCRATCH + 0x40;
    assert_eq!(
        guest.call("InitOnceBeginInitialize", &[retry, 0, pending, 0]),
        1
    );
    assert_eq!(guest.call("InitOnceComplete", &[retry, 4, 8]), 0);
    assert_eq!(guest.error(), ERROR_INVALID_PARAMETER);
    assert_eq!(guest.call("InitOnceComplete", &[retry, 4, 0]), 1);
    assert_eq!(
        guest.call("InitOnceBeginInitialize", &[retry, 0, pending, 0]),
        1
    );
    assert_eq!(guest.u32_at(pending), 1);

    // A second thread would have to wait for the pending initializer.
    assert_eq!(
        guest
            .on(THREAD_A)
            .fails("InitOnceBeginInitialize", &[retry, 0, pending, 0]),
        RUNTIME_LIMIT
    );

    assert_eq!(
        guest.call("InitOnceBeginInitialize", &[once, 2, pending, 0]),
        0
    );
    assert_eq!(guest.error(), ERROR_NOT_SUPPORTED);
    assert_eq!(guest.call("InitOnceBeginInitialize", &[once, 0, 0, 0]), 0);
    assert_eq!(guest.error(), ERROR_INVALID_PARAMETER);
}

#[test]
fn slists_push_and_pop_in_lifo_order_using_the_x64_header_layout() {
    let mut guest = Guest::new();
    let header = SCRATCH;
    let (first, second) = (SCRATCH + 0x100, SCRATCH + 0x200);
    guest.memory.write_bytes(header, &[0xCC; 16]).unwrap();
    assert_eq!(guest.call("InitializeSListHead", &[header]), 0);
    assert_eq!((guest.u64_at(header), guest.u64_at(header + 8)), (0, 0));
    assert_eq!(guest.call("QueryDepthSList", &[header]), 0);

    assert_eq!(guest.call("InterlockedPushEntrySList", &[header, first]), 0);
    assert_eq!(
        guest.call("InterlockedPushEntrySList", &[header, second]),
        first
    );
    assert_eq!(guest.call("QueryDepthSList", &[header]), 2);
    // Depth is the low 16 bits; NextEntry holds the first entry's address
    // shifted right by four in bits 4..63 of the second quadword.
    assert_eq!(guest.u64_at(header) & 0xffff, 2);
    assert_eq!(guest.u64_at(header + 8), second);
    assert_eq!(guest.u64_at(second), first);
    assert_eq!(guest.u64_at(first), 0);

    assert_eq!(guest.call("InterlockedPopEntrySList", &[header]), second);
    assert_eq!(guest.call("QueryDepthSList", &[header]), 1);
    assert_eq!(
        guest.call("InterlockedPushEntrySList", &[header, second]),
        first
    );
    assert_eq!(guest.call("InterlockedFlushSList", &[header]), second);
    assert_eq!(guest.call("QueryDepthSList", &[header]), 0);
    assert_eq!(guest.call("InterlockedPopEntrySList", &[header]), 0);

    // Entries and headers require MEMORY_ALLOCATION_ALIGNMENT (16 bytes).
    assert_eq!(
        guest.fails("InterlockedPushEntrySList", &[header, first + 8]),
        RUNTIME_LIMIT
    );
    assert_eq!(guest.fails("QueryDepthSList", &[header + 8]), RUNTIME_LIMIT);
}

#[test]
fn tls_and_fls_values_are_per_thread_and_cleared_on_reallocation() {
    let mut guest = Guest::new();
    let index = guest.call("TlsAlloc", &[]);
    assert!(index < 1088, "{index}");
    guest.call("SetLastError", &[5]);
    // TlsGetValue clears the last error on success so zero is unambiguous.
    assert_eq!(guest.call("TlsGetValue", &[index]), 0);
    assert_eq!(guest.error(), 0);
    assert_eq!(guest.on(THREAD_A).call("TlsSetValue", &[index, 0xABC0]), 1);
    assert_eq!(guest.on(THREAD_B).call("TlsGetValue", &[index]), 0);
    assert_eq!(guest.call("TlsSetValue", &[index, 0xDEF0]), 1);
    assert_eq!(guest.on(THREAD_A).call("TlsGetValue", &[index]), 0xABC0);
    assert_eq!(guest.on(THREAD_B).call("TlsGetValue", &[index]), 0xDEF0);

    assert_eq!(guest.call("TlsFree", &[index]), 1);
    assert_eq!(guest.call("TlsGetValue", &[index]), 0);
    assert_eq!(guest.error(), ERROR_INVALID_PARAMETER);
    assert_eq!(guest.call("TlsFree", &[index]), 0);
    assert_eq!(guest.error(), ERROR_INVALID_PARAMETER);
    // A newly allocated index reads as zero in every thread.
    let again = guest.call("TlsAlloc", &[]);
    assert_eq!(guest.on(THREAD_A).call("TlsGetValue", &[again]), 0);
    assert_eq!(guest.on(THREAD_B).call("TlsGetValue", &[again]), 0);

    // The documented process limit is 1,088 TLS indexes.
    let mut allocated = 1;
    while guest.call("TlsAlloc", &[]) != u64::from(u32::MAX) {
        allocated += 1;
        assert!(allocated <= 1088);
    }
    assert_eq!(allocated, 1088);

    let plain = guest.call("FlsAlloc", &[0]);
    assert_ne!(plain, u64::from(u32::MAX));
    assert_eq!(guest.call("FlsSetValue", &[plain, 7]), 1);
    assert_eq!(guest.on(THREAD_A).call("FlsGetValue", &[plain]), 0);
    assert_eq!(guest.on(THREAD_B).call("FlsGetValue", &[plain]), 7);
    assert_eq!(guest.call("FlsFree", &[plain]), 1);
    assert_eq!(guest.call("FlsGetValue", &[plain]), 0);
    assert_eq!(guest.error(), ERROR_INVALID_PARAMETER);

    // FlsFree runs the slot's callback once for each nonnull value.
    let with_callback = guest.call("FlsAlloc", &[routine(FLS_ADD)]);
    assert_eq!(
        guest.on(THREAD_A).call("FlsSetValue", &[with_callback, 5]),
        1
    );
    assert_eq!(
        guest.on(THREAD_B).call("FlsSetValue", &[with_callback, 30]),
        1
    );
    let unset = guest.call("FlsAlloc", &[routine(FLS_ADD)]);
    guest.memory.write_u64(LOG, 0).unwrap();
    assert_eq!(guest.call("FlsFree", &[with_callback]), 1);
    assert_eq!(guest.u64_at(LOG), 35);
    assert_eq!(guest.call("FlsGetValue", &[with_callback]), 0);
    assert_eq!(guest.error(), ERROR_INVALID_PARAMETER);
    assert_eq!(guest.call("FlsFree", &[unset]), 1);
    assert_eq!(guest.u64_at(LOG), 35);
}

fn write_i32s(guest: &mut Guest, address: u64, values: &[i32]) {
    for (index, value) in values.iter().enumerate() {
        guest
            .memory
            .write_u32(address + 4 * index as u64, *value as u32)
            .unwrap();
    }
}

fn read_i32s(guest: &Guest, address: u64, count: usize) -> Vec<i32> {
    (0..count)
        .map(|index| guest.u32_at(address + 4 * index as u64) as i32)
        .collect()
}

#[test]
fn qsort_orders_elements_with_a_guest_comparator() {
    let mut guest = Guest::new();
    let array = SCRATCH + 0x100;
    let input = [5, -3, 9, 0, 5, 2, -8, 7, 1, 4, -1];
    write_i32s(&mut guest, array, &input);
    let mut expected = input;
    expected.sort();
    let count = input.len() as u64;
    assert_eq!(guest.call("qsort", &[array, count, 4, routine(COMPARE)]), 0);
    assert_eq!(read_i32s(&guest, array, input.len()), expected);

    // Eight-byte records keyed by their first int: the records move intact.
    let records = SCRATCH + 0x200;
    let keyed = [(3, 30), (1, 10), (2, 20), (0, 0)];
    for (index, (key, tag)) in keyed.iter().enumerate() {
        write_i32s(&mut guest, records + 8 * index as u64, &[*key, *tag]);
    }
    assert_eq!(guest.call("qsort", &[records, 4, 8, routine(COMPARE)]), 0);
    assert_eq!(read_i32s(&guest, records, 8), [0, 0, 1, 10, 2, 20, 3, 30]);

    // qsort_s passes its context as the comparator's first argument.
    write_i32s(&mut guest, array, &input);
    guest.memory.write_u64(COUNTER, 0).unwrap();
    assert_eq!(
        guest.call(
            "qsort_s",
            &[array, count, 4, routine(COMPARE_COUNTING), COUNTER]
        ),
        0
    );
    assert_eq!(read_i32s(&guest, array, input.len()), expected);
    // Any comparison sort of n distinct-enough keys needs at least n - 1 calls.
    let calls = guest.u64_at(COUNTER);
    assert!(calls >= count - 1 && calls <= count * count, "{calls}");

    // Fewer than two elements need no comparison.
    guest.memory.write_u64(COUNTER, 0).unwrap();
    assert_eq!(
        guest.call(
            "qsort_s",
            &[array, 1, 4, routine(COMPARE_COUNTING), COUNTER]
        ),
        0
    );
    assert_eq!(guest.u64_at(COUNTER), 0);
    // A read-only array cannot be sorted in place.
    let text = BASE + u64::from(CODE_RVA);
    assert!(matches!(
        guest.fails("qsort", &[text, 4, 4, routine(COMPARE)]),
        ExitReason::MemoryFault(_)
    ));
    assert_eq!(guest.fails("qsort", &[array, 4, 4, 0]), RUNTIME_LIMIT);
}

#[test]
fn bsearch_returns_the_matching_element_or_null() {
    let mut guest = Guest::new();
    let array = SCRATCH + 0x100;
    let key = SCRATCH + 0x80;
    let sorted = [-8, -3, 0, 2, 5, 9, 14];
    write_i32s(&mut guest, array, &sorted);
    let count = sorted.len() as u64;
    for (index, value) in sorted.iter().enumerate() {
        write_i32s(&mut guest, key, &[*value]);
        assert_eq!(
            guest.call("bsearch", &[key, array, count, 4, routine(COMPARE)]),
            array + 4 * index as u64
        );
    }
    for missing in [-9, 1, 15] {
        write_i32s(&mut guest, key, &[missing]);
        assert_eq!(
            guest.call("bsearch", &[key, array, count, 4, routine(COMPARE)]),
            0
        );
    }
    assert_eq!(
        guest.call("bsearch", &[key, array, 0, 4, routine(COMPARE)]),
        0
    );

    write_i32s(&mut guest, key, &[14]);
    guest.memory.write_u64(COUNTER, 0).unwrap();
    assert_eq!(
        guest.call(
            "bsearch_s",
            &[key, array, count, 4, routine(COMPARE_COUNTING), COUNTER]
        ),
        array + 24
    );
    // A binary search of seven elements needs at most three comparisons.
    let calls = guest.u64_at(COUNTER);
    assert!((1..=3).contains(&calls), "{calls}");
}

#[test]
fn initializer_tables_run_in_order_and_stop_on_failure() {
    let mut guest = Guest::new();
    let table = SCRATCH + 0x300;
    let write_table = |guest: &mut Guest, entries: &[u64]| {
        for (index, entry) in entries.iter().enumerate() {
            guest
                .memory
                .write_u64(table + 8 * index as u64, *entry)
                .unwrap();
        }
        table + 8 * entries.len() as u64
    };

    // Null entries are skipped.
    let end = write_table(
        &mut guest,
        &[routine(APPEND_1), 0, routine(APPEND_2), routine(APPEND_3)],
    );
    guest.memory.write_u64(LOG, 0).unwrap();
    assert_eq!(guest.call("_initterm", &[table, end]), 0);
    assert_eq!(guest.u64_at(LOG), 123);
    assert_eq!(guest.call("_initterm", &[table, table]), 0);
    assert_eq!(guest.u64_at(LOG), 123);

    // _initterm_e returns the first nonzero result and runs nothing after it.
    let end = write_table(
        &mut guest,
        &[routine(APPEND_2), routine(FAIL), routine(APPEND_3)],
    );
    guest.memory.write_u64(LOG, 0).unwrap();
    assert_eq!(guest.call("_initterm_e", &[table, end]), 7);
    assert_eq!(guest.u64_at(LOG), 2);
    let end = write_table(&mut guest, &[routine(APPEND_3), routine(APPEND_1)]);
    guest.memory.write_u64(LOG, 0).unwrap();
    assert_eq!(guest.call("_initterm_e", &[table, end]), 0);
    assert_eq!(guest.u64_at(LOG), 31);

    // An initializer that does not preserve RBX breaks the Win64 contract.
    let end = write_table(&mut guest, &[routine(CLOBBER_RBX)]);
    assert_eq!(
        guest.fails("_initterm", &[table, end]),
        ExitReason::ServiceAbiViolation
    );
    // Reversed and misaligned tables are rejected.
    assert_eq!(guest.fails("_initterm", &[end, table]), RUNTIME_LIMIT);
    assert_eq!(guest.fails("_initterm", &[table, table + 4]), RUNTIME_LIMIT);
}

#[test]
fn init_once_execute_once_runs_the_initializer_exactly_once() {
    let mut guest = Guest::new();
    let once = SCRATCH;
    let context = SCRATCH + 0x10;
    guest.memory.write_u64(COUNTER, 0).unwrap();
    let execute = |guest: &mut Guest, once: u64, initializer: usize, parameter: u64| {
        guest.call(
            "InitOnceExecuteOnce",
            &[once, routine(initializer), parameter, context],
        )
    };
    assert_eq!(execute(&mut guest, once, INIT_ONCE_STORE, 0x5000), 1);
    assert_eq!(guest.u64_at(context), 0x5000);
    assert_eq!(guest.u64_at(COUNTER), 1);
    guest.memory.write_u64(context, 0).unwrap();
    assert_eq!(
        execute(guest.on(THREAD_B), once, INIT_ONCE_STORE, 0x6000),
        1
    );
    assert_eq!(guest.u64_at(context), 0x5000);
    assert_eq!(guest.u64_at(COUNTER), 1);

    // The completed context is visible through InitOnceBeginInitialize.
    let pending = SCRATCH + 0x18;
    assert_eq!(
        guest.call("InitOnceBeginInitialize", &[once, 1, pending, context]),
        1
    );
    assert_eq!(guest.u32_at(pending), 0);

    // A failed initializer leaves the object uninitialized for a retry.
    let retry = SCRATCH + 0x40;
    assert_eq!(execute(&mut guest, retry, INIT_ONCE_FAIL, 0), 0);
    assert_eq!(guest.u64_at(retry), 0);
    assert_eq!(execute(&mut guest, retry, INIT_ONCE_STORE, 0x7000), 1);
    assert_eq!(guest.u64_at(context), 0x7000);
    assert_eq!(guest.u64_at(COUNTER), 2);

    // A context using the reserved low bits cannot be stored.
    let reserved = SCRATCH + 0x60;
    assert_eq!(execute(&mut guest, reserved, INIT_ONCE_STORE, 0x7001), 0);
    assert_eq!(guest.error(), ERROR_INVALID_PARAMETER);
    assert_eq!(guest.u64_at(reserved), 0);
}
