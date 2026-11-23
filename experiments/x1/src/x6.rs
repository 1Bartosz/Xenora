// SPDX-License-Identifier: GPL-3.0-or-later
//! X6 — multi-function call demonstration using the generalised PE32+ loader and
//! extended x86-64 interpreter.
//!
//! # Program structure
//!
//! X6 is an original synthetic PE32+ test executable with two sections:
//!
//! - `.text` (RX) — 40 bytes of x86-64 machine code with three functions:
//!   `entry`, `count_down`, and `double_store`.
//! - `.data` (RW) — 8 bytes of writable guest memory (sentinel fill 0xA5).
//!
//! The program is loaded through the generalised [`loader`] and executed by the
//! extended [`interp`] interpreter.
//!
//! # Code layout (derived independently from the fixture generator)
//!
//! ```text
//! ; ── entry (offset 0) ─────────────────────────────────────────────────────
//! 0:  B8 01 00 00 00    MOV EAX, 1          load service-1 ID
//! 5:  0F 0B             UD2                 service 1: EAX = RCX (= N)
//! 7:  E8 06 00 00 00    CALL count_down     next=12, rel=6, target=18
//! 12: E8 0C 00 00 00    CALL double_store   next=17, rel=12, target=29
//! 17: C3                RET                 return from the root frame
//!
//! ; ── count_down (offset 18) ───────────────────────────────────────────────
//! 18: 83 F8 00          CMP EAX, 0          is EAX zero?
//! 21: 74 05             JE  +5  → 28        if zero, skip loop body
//! 23: 83 E8 01          SUB EAX, 1          EAX--
//! 26: EB F6             JMP -10 → 18        next=28, rel=-10, target=18
//! 28: C3                RET
//!
//! ; ── double_store (offset 29) ─────────────────────────────────────────────
//! 29: B8 02 00 00 00    MOV EAX, 2          load service-2 ID
//! 34: 0F 0B             UD2                 service 2: EAX = RCX*2 (wrapping)
//! 36: 48 89 07          MOV [RDI], RAX      store result to .data
//! 39: C3                RET
//! ```
//!
//! # Services
//!
//! | ID | Name       | Returns          |
//! |----|------------|------------------|
//! | 1  | identity   | RCX (unchanged)  |
//! | 2  | double     | RCX × 2 (wrap)   |
//!
//! # Independent expected outputs
//!
//! Derived from the program listing above, before running the code:
//!
//! | N | count_down iterations | service2 result | `.data[0..8]`              |
//! |---|-----------------------|-----------------|----------------------------|
//! | 0 | 0 (JE taken at 21)    | 0×2 = 0         | `[0,0,0,0,0,0,0,0]`        |
//! | 1 | 1                     | 1×2 = 2         | `[2,0,0,0,0,0,0,0]`        |
//! | 2 | 2                     | 2×2 = 4         | `[4,0,0,0,0,0,0,0]`        |
//! | 3 | 3                     | 3×2 = 6         | `[6,0,0,0,0,0,0,0]`        |
//!
//! [`loader`]: crate::loader
//! [`interp`]: crate::interp

use crate::interp::{self, FaultKind, State};
use crate::loader;

const IMAGE: &[u8] = include_bytes!("../fixtures/x6.exe");

/// Execution budget sufficient for N=3.
/// N=3 needs: 2 (entry) + ~12 (count_down: 3 iterations × 3 steps + 2 terminal) + 4 (double_store) = ~20.
/// Budget of 64 is comfortable and well below MAX_STEPS.
const X6_BUDGET: usize = 64;

/// Stack parameters.
const STACK_BASE: u64 = 0x7FFF_F000;
const STACK_SIZE: usize = 0x2000;

// ── service handlers ─────────────────────────────────────────────────────────

/// Service 1 — identity: returns RCX unchanged.
fn service_identity(state: &State) -> Result<u64, FaultKind> {
    Ok(state.gpr[1]) // RCX
}

/// Service 2 — double: returns RCX × 2 (wrapping).
fn service_double(state: &State) -> Result<u64, FaultKind> {
    Ok(state.gpr[1].wrapping_mul(2)) // RCX
}

/// Build a combined service dispatcher for X6.
fn make_service() -> impl FnMut(&State) -> Result<u64, FaultKind> {
    move |state: &State| {
        let id = state.gpr[0] & 0xFFFF_FFFF; // EAX
        match id {
            1 => service_identity(state),
            2 => service_double(state),
            _ => Err(FaultKind::UnknownService),
        }
    }
}

// ── public API ───────────────────────────────────────────────────────────────

#[derive(Debug)]
struct Execution {
    data: Vec<u8>,
    retired: usize,
}

fn execute(n: u64) -> Result<Execution, String> {
    let mut loaded = loader::load(IMAGE).map_err(|error| format!("PE load: {error:?}"))?;
    let rsp = loaded
        .mem
        .alloc_stack(STACK_BASE, STACK_SIZE)
        .ok_or("stack allocation failed")?;
    let data_va = loaded.image_base + 0x2000;
    let mut state = State::new(loaded.entry);
    state.gpr[1] = n;
    state.gpr[7] = data_va;
    state.gpr[4] = rsp;

    let mut service = make_service();
    let mut retired = 0;
    interp::run(
        &mut loaded.mem,
        &mut state,
        X6_BUDGET,
        Some(&mut service),
        |_| retired += 1,
    )
    .map_err(|fault| format!("guest fault {:?} at RIP {:#x}", fault.kind, fault.rip))?;
    let data = loaded
        .mem
        .read_bytes(data_va, 8)
        .map_err(|error| format!("output read: {error:?}"))?;
    Ok(Execution { data, retired })
}

/// Non-test CLI entrypoint: demonstrates generalised loader + extended interpreter.
pub(crate) fn run() -> std::process::ExitCode {
    println!("X6: generalised PE32+ loader with CALL/RET and guest virtual memory");
    let mut all_ok = true;
    for n in [0u64, 1, 2, 3] {
        let Execution { data, retired } = match execute(n) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("  N={n}: execution failed: {e}");
                return std::process::ExitCode::FAILURE;
            }
        };
        // Independent expected value: service2 returns RCX*2.
        let expected_u64 = n.wrapping_mul(2);
        let expected: Vec<u8> = expected_u64.to_le_bytes().to_vec();
        let expected_retired = 12 + 4 * n as usize;
        let pass = data == expected && retired == expected_retired;
        let status = if pass { "PASS" } else { "FAIL" };
        println!(
            "  N={n}: {status}  data={data:02x?}  expected={expected:02x?}  retired={retired}/{expected_retired}"
        );
        if !pass {
            all_ok = false;
        }
    }
    if all_ok {
        println!("X6: all cases passed");
        std::process::ExitCode::SUCCESS
    } else {
        eprintln!("X6: one or more cases failed");
        std::process::ExitCode::FAILURE
    }
}

// ── tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    use crate::interp::Fault;

    // Expected values are derived from the program listing in the module
    // docstring, NOT from running the code first.
    //
    // The derivation is:
    //   service1(rcx=N) → EAX = N
    //   count_down: decrements EAX to 0 (N iterations for N > 0, 0 for N = 0)
    //   After count_down: EAX = 0.
    //   double_store: service2(rcx=N) → EAX = N*2; store as little-endian u64.
    //   data[0..8] = (N*2).to_le_bytes()

    #[test]
    fn n0_skips_countdown_stores_zero() {
        let execution = execute(0).unwrap();
        assert_eq!(execution.data, [0, 0, 0, 0, 0, 0, 0, 0]);
    }

    #[test]
    fn n1_one_countdown_iteration_stores_2() {
        // N=1: svc1→1; count_down: CMP 1,0→ZF=0→JE not taken; SUB→0; JMP→18;
        //              CMP 0,0→ZF=1→JE→28; RET.
        // double_store: svc2(rcx=1)→2; STORE 2.
        let execution = execute(1).unwrap();
        assert_eq!(execution.data, [2, 0, 0, 0, 0, 0, 0, 0]);
    }

    #[test]
    fn n2_two_countdown_iterations_stores_4() {
        // N=2: svc1→2; count_down: 2 SUB iterations; svc2(rcx=2)→4; STORE 4.
        let execution = execute(2).unwrap();
        assert_eq!(execution.data, [4, 0, 0, 0, 0, 0, 0, 0]);
    }

    #[test]
    fn n3_three_countdown_iterations_stores_6() {
        // N=3: svc1→3; count_down: 3 SUB iterations; svc2(rcx=3)→6; STORE 6.
        let execution = execute(3).unwrap();
        assert_eq!(execution.data, [6, 0, 0, 0, 0, 0, 0, 0]);
    }

    #[test]
    fn service_identity_returns_rcx() {
        let state = State::new(0);
        let mut s = state.clone();
        s.gpr[1] = 42; // RCX
        assert_eq!(service_identity(&s).unwrap(), 42);
        assert_eq!(service_identity(&State::new(0)).unwrap(), 0);
    }

    #[test]
    fn service_double_returns_rcx_times_2() {
        let mut s = State::new(0);
        s.gpr[1] = 7;
        assert_eq!(service_double(&s).unwrap(), 14);
    }

    #[test]
    fn service_double_wraps_on_overflow() {
        let mut s = State::new(0);
        s.gpr[1] = u64::MAX;
        assert_eq!(service_double(&s).unwrap(), u64::MAX.wrapping_mul(2));
    }

    #[test]
    fn budget_too_small_returns_budget_fault() {
        let mut loaded = loader::load(IMAGE).unwrap();
        let rsp = loaded.mem.alloc_stack(STACK_BASE, STACK_SIZE).unwrap();
        let mut state = State::new(loaded.entry);
        state.gpr[1] = 0;
        state.gpr[7] = loaded.image_base + 2 * 0x1000;
        state.gpr[4] = rsp;
        let mut svc = make_service();
        let result = interp::run(
            &mut loaded.mem,
            &mut state,
            1, // too small
            Some(&mut svc),
            |_| {},
        );
        assert!(
            matches!(
                result,
                Err(Fault {
                    kind: FaultKind::Budget,
                    ..
                })
            ),
            "expected Budget fault, got {result:?}"
        );
    }

    #[test]
    fn fixture_retires_guest_calls_and_services_in_order() {
        let mut loaded = loader::load(IMAGE).unwrap();
        let mut state = State::new(loaded.entry);
        let stack_top = loaded.mem.alloc_stack(STACK_BASE, STACK_SIZE).unwrap();
        state.gpr[4] = stack_top;
        state.gpr[1] = 3;
        state.gpr[7] = loaded.image_base + 0x2000;
        let mut dispatched = Vec::new();
        let mut retired = Vec::new();
        let mut service = make_service();
        let mut record_service = |state: &State| {
            dispatched.push(state.gpr[0]);
            service(state)
        };

        interp::run(
            &mut loaded.mem,
            &mut state,
            24,
            Some(&mut record_service),
            |state| retired.push(state.rip - loaded.entry),
        )
        .unwrap();

        assert_eq!(dispatched, [1, 2]);
        assert_eq!(
            retired,
            [
                5, 7, 18, 21, 23, 26, 18, 21, 23, 26, 18, 21, 23, 26, 18, 21, 28, 12, 29, 34, 36,
                39, 17, 17,
            ]
        );
        assert_eq!(state.gpr[4], stack_top);
        assert_eq!(state.gpr[0], 6);
        assert_eq!(loaded.mem.load_u64(state.gpr[7]), Ok(6));
    }

    #[test]
    fn fixture_loads_through_generalised_loader() {
        // Verify the X6 fixture loads without error using the generalized loader.
        let loaded = loader::load(IMAGE).unwrap();
        assert_eq!(loaded.image_base, 0x1_4000_0000);
        assert_eq!(loaded.entry, 0x1_4000_1000);
    }

    #[test]
    fn data_section_has_sentinel_fill_before_execution() {
        // The .data section should be filled with 0xA5 before execution runs.
        let loaded = loader::load(IMAGE).unwrap();
        let data_va = loaded.image_base + 2 * 0x1000;
        let data = loaded.mem.read_bytes(data_va, 8).unwrap();
        assert_eq!(
            data, [0xA5; 8],
            "initial sentinel should be preserved before run"
        );
    }

    #[test]
    fn execution_isolated_across_calls() {
        // Running with different N values must produce independent results.
        for n in [0u64, 1, 2, 3] {
            let execution = execute(n).unwrap();
            assert_eq!(execution.data, (n * 2).to_le_bytes(), "N={n}");
            assert_eq!(execution.retired, 12 + 4 * n as usize, "N={n}");
        }
    }

    // ── Generalised loader integration tests ─────────────────────────────────

    #[test]
    fn loader_rejects_x6_with_import_directory() {
        // Patch the import data directory to non-zero; loader must reject it.
        let mut image = IMAGE.to_vec();
        // PE32+ import directory at optional header + 112 + 8.
        image[0x110] = 1; // non-zero RVA
        assert_eq!(
            loader::load(&image).unwrap_err(),
            loader::LoadError::Directory
        );
    }

    #[test]
    fn loader_accepts_alternate_section_layout() {
        // Build a PE with three sections (text, data, rdata) using the test builder.
        use crate::loader::tests::build_pe;
        let mut image = build_pe(
            0x1_8000_0000,
            &[
                (b".text\0\0\0", &[0x90u8, 0xC3], 0x6000_0020), // NOP; RET
                (b".data\0\0\0", &[0u8; 4], 0xC000_0040),
                (b".rdata\0\0", &[0xFFu8; 4], 0x4000_0040),
            ],
        );
        // Relocate the header, put the read-only section in a different
        // page, and start at the second byte of .text (RET).
        image.copy_within(0x80..0x200, 0x40);
        image[0x3C..0x40].copy_from_slice(&0x40u32.to_le_bytes());
        image[0x68..0x6C].copy_from_slice(&0x1001u32.to_le_bytes());
        image[0x90..0x94].copy_from_slice(&0x6000u32.to_le_bytes());
        image[0x1A4..0x1A8].copy_from_slice(&0x5000u32.to_le_bytes());
        let mut loaded = loader::load(&image).unwrap();
        assert_eq!(loaded.image_base, 0x1_8000_0000);
        assert_eq!(loaded.entry, 0x1_8000_1001);
        let mut state = State::new(loaded.entry);
        state.gpr[4] = loaded.mem.alloc_stack(STACK_BASE, STACK_SIZE).unwrap();
        interp::run(&mut loaded.mem, &mut state, 1, None, |_| {}).unwrap();
        // .rdata should be readable but not writable.
        assert!(loaded.mem.load_u8(0x1_8000_5000).is_ok());
        assert!(loaded.mem.store_u8(0x1_8000_5000, 0).is_err());
    }

    // ── Memory permission tests via the loaded image ──────────────────────────

    #[test]
    fn code_section_not_writable() {
        let mut loaded = loader::load(IMAGE).unwrap();
        // .text at 0x140001000 must not be writable.
        assert!(
            loaded.mem.store_u8(0x1_4000_1000, 0).is_err(),
            ".text must not be writable"
        );
    }

    #[test]
    fn data_section_not_executable() {
        let loaded = loader::load(IMAGE).unwrap();
        // .data at 0x140002000 must not be executable.
        assert!(
            loaded.mem.fetch(0x1_4000_2000, 4).is_err(),
            ".data must not be executable"
        );
    }
}
