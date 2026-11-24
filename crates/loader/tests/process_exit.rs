// SPDX-License-Identifier: GPL-3.0-or-later

//! Process exit (issue #329): `ExitProcess`, a returning entry point, and the
//! rule that an exited guest runs nothing further.
//!
//! Expected behaviour is from the public Win32 documentation for
//! `ExitProcess` (processthreadsapi.h): it does not return, the status is a
//! `UINT`, and the process's handles are closed. The guest code is original.

mod support;

use std::sync::Arc;
use support::*;
use xenora_loader::cpu::{CpuState, ExitReason};
use xenora_loader::import_table::ImportName;
use xenora_loader::imports::{ImportRegistry, Win64Args};
use xenora_loader::memory::GuestMemory;
use xenora_loader::services::{MockClock, ServicesState, register_standard_services};
use xenora_loader::startup::{StartupLayout, run_synthetic_startup};

const KERNEL32: &str = "kernel32.dll";
const THREADS: &str = "api-ms-win-core-processthreads-l1-1-0.dll";
const UCRT: &str = "ucrtbase.dll";

const IMPORTS: &[(&str, &str)] = &[
    (KERNEL32, "ExitProcess"),
    (KERNEL32, "SetLastError"),
    (KERNEL32, "CreateFileW"),
    (KERNEL32, "CreateEventW"),
    (UCRT, "qsort"),
];
const ALIAS: &[(&str, &str)] = &[(THREADS, "ExitProcess")];

/// Guest address of the stub `call [slot]; hlt` for import `index`.
fn stub(index: usize) -> u64 {
    BASE + u64::from(CODE_RVA) + 16 * index as u64
}

#[test]
fn exit_process_stops_the_guest_with_its_whole_status() {
    // The status is a 32-bit UINT: values above 255 survive, and the upper
    // half of RCX is not part of it.
    for (argument, status) in [
        (0, 0),
        (1, 1),
        (150, 150),
        (0x1234_5678, 0x1234_5678),
        (0xFFFF_FFFF, 0xFFFF_FFFF),
        (0xAAAA_BBBB_0000_002A, 0x2A),
    ] {
        for imports in [IMPORTS, ALIAS] {
            let mut guest = Guest::new(imports, &[]);
            assert_eq!(guest.state.exit_status(), None);
            // The stub is `call [ExitProcess]; hlt`. A service that returned
            // would reach the HLT and report Halt.
            let (_, exit) = guest.run("ExitProcess", &[argument]);
            assert_eq!(exit, ExitReason::ProcessExit(status), "{argument:#x}");
            assert_eq!(guest.state.exit_status(), Some(status), "{argument:#x}");
        }
    }
}

#[test]
fn an_exited_guest_runs_nothing_further() {
    let mut guest = Guest::new(IMPORTS, &[]);
    assert_eq!(guest.run("ExitProcess", &[3]).1, ExitReason::ProcessExit(3));
    // Neither another service nor a second exit takes effect: the engine
    // reports the first exit again without executing anything.
    for (symbol, argument) in [("SetLastError", 5), ("ExitProcess", 9)] {
        let (_, exit) = guest.run(symbol, &[argument]);
        assert_eq!(exit, ExitReason::ProcessExit(3), "{symbol}");
    }
    assert_eq!(guest.state.last_error(THREAD_A), 0);
    assert_eq!(guest.state.exit_status(), Some(3));
}

#[test]
fn the_first_exit_decides_the_status() {
    // Two calls of the service itself, with no engine in between to stop the
    // second one.
    let state = Arc::new(ServicesState::new(Arc::new(MockClock::new(0))));
    let mut registry = ImportRegistry::new();
    register_standard_services(&mut registry, Arc::clone(&state));
    let exit = &registry
        .resolve(KERNEL32, &ImportName::Name("ExitProcess".into()))
        .expect("ExitProcess is registered")
        .handler;
    let mut cpu = CpuState::new(0, 0);
    let mut memory = GuestMemory::new();
    for status in [41, 42] {
        let mut args = Win64Args::new(&cpu);
        args.rcx = status;
        assert_eq!(
            exit(&args, &mut cpu, &mut memory),
            Err(ExitReason::ProcessExit(41))
        );
    }
    assert_eq!(state.exit_status(), Some(41));
}

#[test]
fn exit_from_inside_a_service_callback_ends_the_process() {
    // A comparator for qsort that exits instead of comparing:
    //   sub rsp, 0x28
    //   mov ecx, 9
    //   call <ExitProcess stub>
    //   mov byte ptr [SCRATCH], 1      ; must never run
    //   add rsp, 0x28
    //   ret
    let call_end = routine(0) + 14;
    let displacement = (stub(0).wrapping_sub(call_end) as u32).to_le_bytes();
    let mut comparator = vec![0x48, 0x83, 0xEC, 0x28, 0xB9, 9, 0, 0, 0, 0xE8];
    comparator.extend(displacement);
    comparator.extend([0xC6, 0x04, 0x25]);
    comparator.extend((SCRATCH as u32).to_le_bytes());
    comparator.extend([0x01, 0x48, 0x83, 0xC4, 0x28, 0xC3]);

    let mut guest = Guest::new(IMPORTS, &[comparator]);
    let items = SCRATCH + 0x100;
    guest.memory.write_u64(items, 2).unwrap();
    guest.memory.write_u64(items + 8, 1).unwrap();
    let (_, exit) = guest.run("qsort", &[items, 2, 8, routine(0)]);
    assert_eq!(exit, ExitReason::ProcessExit(9));
    assert_eq!(guest.memory.read_u8(SCRATCH).unwrap(), 0);
    // The sort never resumed, so the items are as the guest left them.
    assert_eq!((guest.u64_at(items), guest.u64_at(items + 8)), (2, 1));
    assert_eq!(guest.state.exit_status(), Some(9));
}

#[test]
fn exit_closes_every_handle_and_keeps_the_files() {
    let mut guest = Guest::new(IMPORTS, &[]);
    let name: Vec<u8> = "C:\\exit.bin\0"
        .encode_utf16()
        .flat_map(u16::to_le_bytes)
        .collect();
    guest.memory.write_bytes(SCRATCH, &name).unwrap();
    // CreateFileW(name, GENERIC_READ | GENERIC_WRITE, no sharing, null,
    // CREATE_ALWAYS, 0, null) and an unnamed event.
    let file = guest.call("CreateFileW", &[SCRATCH, 0xC000_0000, 0, 0, 2, 0, 0]);
    assert_ne!(file, u64::MAX);
    assert_ne!(guest.call("CreateEventW", &[0, 1, 0, 0]), 0);
    assert_eq!(guest.state.open_handle_count(), 2);

    assert_eq!(guest.run("ExitProcess", &[0]).1, ExitReason::ProcessExit(0));
    assert_eq!(guest.state.open_handle_count(), 0);
    // The drive is not the process: the file it created is still there.
    assert_eq!(guest.state.file_contents("C:\\exit.bin"), Some(Vec::new()));
    assert_eq!(guest.state.file_contents("C:\\absent.bin"), None);
}

/// A PE32+ image with one executable section and no imports whose entry
/// point is `mov eax, status; ret`.
fn returning_image(status: u32) -> Vec<u8> {
    let mut pe = vec![0u8; 0x400];
    let mut put = |at: usize, bytes: &[u8]| pe[at..at + bytes.len()].copy_from_slice(bytes);
    put(0, b"MZ");
    put(0x3C, &0x80u32.to_le_bytes());
    put(0x80, b"PE\0\0");
    put(0x84, &0x8664u16.to_le_bytes()); // AMD64
    put(0x86, &1u16.to_le_bytes()); // one section
    put(0x94, &240u16.to_le_bytes()); // optional header size
    put(0x96, &0x0022u16.to_le_bytes()); // executable, large-address-aware
    let optional = 0x98;
    put(optional, &0x20Bu16.to_le_bytes()); // PE32+
    put(optional + 16, &0x1000u32.to_le_bytes()); // entry point
    put(optional + 24, &0x1_4000_0000u64.to_le_bytes()); // image base
    put(optional + 32, &0x1000u32.to_le_bytes()); // section alignment
    put(optional + 36, &0x200u32.to_le_bytes()); // file alignment
    put(optional + 56, &0x2000u32.to_le_bytes()); // size of image
    put(optional + 60, &0x200u32.to_le_bytes()); // size of headers
    put(optional + 108, &16u32.to_le_bytes()); // data directories
    let section = optional + 240;
    put(section, b".text");
    put(section + 8, &0x200u32.to_le_bytes()); // virtual size
    put(section + 12, &0x1000u32.to_le_bytes()); // RVA
    put(section + 16, &0x200u32.to_le_bytes()); // raw size
    put(section + 20, &0x200u32.to_le_bytes()); // raw offset
    put(section + 36, &0x6000_0020u32.to_le_bytes()); // code, execute, read
    put(0x200, &[0xB8]);
    put(0x201, &status.to_le_bytes());
    put(0x205, &[0xC3]);
    pe
}

fn layout() -> StartupLayout {
    StartupLayout {
        image_base: 0x1_4000_0000,
        stack_top: 0x2_0001_0000,
        tls_base: 0x3_0000_0000,
    }
}

#[test]
fn a_returning_entry_point_exits_with_eax() {
    // With no ExitProcess registered the return still ends the process.
    let bare = run_synthetic_startup(
        &returning_image(0x1234_5678),
        layout(),
        ImportRegistry::new(),
        100,
    )
    .unwrap();
    assert_eq!(
        bare.execution.exit_reason,
        ExitReason::ProcessExit(0x1234_5678)
    );
    // `mov eax, imm32` and `ret`, then nothing.
    assert_eq!(bare.execution.instructions_executed, 2);

    // With the standard services it goes through ExitProcess, so the status
    // is recorded where a host can read it.
    let state = Arc::new(ServicesState::new(Arc::new(MockClock::new(0))));
    let mut registry = ImportRegistry::new();
    register_standard_services(&mut registry, Arc::clone(&state));
    let standard = run_synthetic_startup(&returning_image(77), layout(), registry, 100).unwrap();
    assert_eq!(standard.execution.exit_reason, ExitReason::ProcessExit(77));
    assert_eq!(state.exit_status(), Some(77));
}
