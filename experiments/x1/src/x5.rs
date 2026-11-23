// SPDX-License-Identifier: GPL-3.0-or-later
//! X5 — original synthetic PE32+ guest fixture: countdown-loop with two-service dispatch.
//!
//! Code layout (30 bytes, two sections: .text RX and .data RW):
//!
//! ```text
//! [0..5]  b8 01 00 00 00  MOV EAX,1          → load service-1 ID
//! [5..7]  0f 0b           UD2                 → service 1 (identity): EAX = RCX
//! [7..10] 83 f8 00        CMP EAX,0           → is N zero?
//! [10..12] 74 06          JE  +6  → [18]      → skip loop if N==0
//! [12..15] 83 e8 01       SUB EAX,1           → N--
//! [15..18] 83 f8 00       CMP EAX,0           → check again
//! [18..20] 75 f3          JNE -13 → [7]       → backward branch to CMP
//! [20..25] b8 02 00 00 00 MOV EAX,2           → load service-2 ID
//! [25..27] 0f 0b          UD2                 → service 2 (double): EAX = RCX*2
//! [27..30] 48 89 07       STORE [RDI]         → write result to guest .data
//! ```
//!
//! Independent expected outputs (derived before running the fixture):
//! - N=0 → data = [0, 0, 0, 0, 0, 0, 0, 0]
//! - N=1 → data = [2, 0, 0, 0, 0, 0, 0, 0]
//! - N=2 → data = [4, 0, 0, 0, 0, 0, 0, 0]
//! - N=3 → data = [6, 0, 0, 0, 0, 0, 0, 0]

use crate::cpu::{Fault, FaultKind};
use crate::pe::{self, LoadError, ServiceEntry, ServiceRegistry};

const IMAGE: &[u8] = include_bytes!("../fixtures/x5.exe");

/// Step budget sufficient for N=3 (needs 20 steps); bounded well below MAX_STEPS=32.
const X5_BUDGET: usize = 24;

// ── service handlers ────────────────────────────────────────────────────────

/// Service 1 — identity: returns RCX (the initial argument N) unchanged.
/// Proves dispatch by EAX=1; transfers the countdown starting value into EAX.
fn service_identity(state: &crate::cpu::State) -> Result<u64, FaultKind> {
    Ok(state.gpr[1]) // RCX
}

/// Service 2 — double: returns RCX×2 (wrapping multiply).
/// Called once after the countdown loop exits; result is stored to guest .data.
fn service_double(state: &crate::cpu::State) -> Result<u64, FaultKind> {
    Ok(state.gpr[1].wrapping_mul(2)) // RCX*2
}

fn build_registry() -> ServiceRegistry {
    let mut registry = ServiceRegistry::new();
    registry.register(
        1,
        ServiceEntry {
            name: "identity",
            handler: Box::new(service_identity),
        },
    );
    registry.register(
        2,
        ServiceEntry {
            name: "double",
            handler: Box::new(service_double),
        },
    );
    registry
}

// ── public API ───────────────────────────────────────────────────────────────

/// Load the X5 fixture, run it with `n` as the initial RCX argument, and
/// return `(data_section, run_result)`.  The image is re-loaded on each call
/// so that individual test cases are independent.
pub(crate) fn execute(n: u64) -> Result<([u8; 8], Result<(), Fault>), LoadError> {
    let mut image = pe::load(IMAGE)?;
    let mut state = image.initial(n);
    let mut registry = build_registry();
    let result = image.run_with_registry(&mut state, X5_BUDGET, &mut registry, |_| {});
    Ok((image.data, result))
}

/// Non-test CLI entrypoint: demonstrates registry dispatch in a normal build.
/// Runs X5 for N=1,2,3, prints a trace, verifies independently derived
/// expected outputs, and returns `ExitCode::SUCCESS` iff all pass.
pub(crate) fn run() -> std::process::ExitCode {
    println!("X5: two-service dispatch with countdown loop");
    let mut all_ok = true;
    for n in [0u64, 1, 2, 3] {
        let (data, result) = match execute(n) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("  N={n}: load failed: {e:?}");
                return std::process::ExitCode::FAILURE;
            }
        };
        // Independent expected value: service2 returns RCX*2 and STORE writes
        // it as a little-endian u64 starting at byte 0 of the data section.
        let expected_u64: u64 = n.wrapping_mul(2);
        let expected_data = expected_u64.to_le_bytes();
        let pass = result.is_ok() && data == expected_data;
        let status = if pass { "PASS" } else { "FAIL" };
        println!(
            "  N={n}: {status}  data={data:02x?}  expected={expected_data:02x?}  run={result:?}"
        );
        if !pass {
            all_ok = false;
        }
    }
    if all_ok {
        println!("X5: all cases verified");
        std::process::ExitCode::SUCCESS
    } else {
        eprintln!("X5: one or more cases failed");
        std::process::ExitCode::FAILURE
    }
}

// ── tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cpu::FaultKind;

    // Expected values are derived independently from the program listing above,
    // not by running the code first.  RCX is set to N before execution starts;
    // service 1 returns RCX unchanged (→ EAX=N); the loop decrements EAX to 0;
    // service 2 returns RCX*2; STORE writes that u64 to .data[0..8].

    #[test]
    fn n0_skips_loop_calls_both_services_stores_0() {
        // N=0: service1→0, CMP 0,0 sets ZF=1, JE taken→[18],
        //      JNE at [18] ZF=1 falls through→[20], service2(rcx=0)→0, STORE 0.
        let (data, result) = execute(0).unwrap();
        result.expect("N=0 must not fault");
        assert_eq!(data, [0, 0, 0, 0, 0, 0, 0, 0]);
    }

    #[test]
    fn n1_calls_both_services_stores_2() {
        // N=1: service1→1, JE falls, SUB→0, CMP ZF=1, JNE falls→[20],
        //      service2(rcx=1)→2, STORE 2.
        let (data, result) = execute(1).unwrap();
        result.expect("N=1 must not fault");
        assert_eq!(data, [2, 0, 0, 0, 0, 0, 0, 0]);
    }

    #[test]
    fn n2_calls_both_services_stores_4() {
        // N=2: service1→2, loop once (SUB→1; CMP ZF=0; JNE taken; SUB→0; CMP ZF=1;
        //      JNE falls), service2(rcx=2)→4, STORE 4.
        let (data, result) = execute(2).unwrap();
        result.expect("N=2 must not fault");
        assert_eq!(data, [4, 0, 0, 0, 0, 0, 0, 0]);
    }

    #[test]
    fn n3_completes_within_budget_stores_6() {
        // N=3: service1→3, loop twice, service2(rcx=3)→6, STORE 6.
        // Requires 20 steps; budget=24, MAX_STEPS=32 → allowed.
        let (data, result) = execute(3).unwrap();
        result.expect("N=3 must not fault");
        assert_eq!(data, [6, 0, 0, 0, 0, 0, 0, 0]);
    }

    #[test]
    fn budget_too_small_returns_budget_fault() {
        // With budget=5 and N=3 the loop cannot complete; must return Budget fault.
        let mut image = pe::load(IMAGE).unwrap();
        let mut state = image.initial(3);
        let mut registry = build_registry();
        let result = image.run_with_registry(&mut state, 5, &mut registry, |_| {});
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
    fn service_identity_returns_rcx() {
        // Unit-test service 1 directly: RCX=42 → returns 42.
        let mut state = crate::cpu::State {
            gpr: [0; 16],
            rip: 0,
            rflags: 2,
        };
        state.gpr[1] = 42; // RCX
        assert_eq!(service_identity(&state).unwrap(), 42);
    }

    #[test]
    fn service_double_returns_rcx_times_2() {
        // Unit-test service 2 directly: RCX=7 → returns 14.
        let mut state = crate::cpu::State {
            gpr: [0; 16],
            rip: 0,
            rflags: 2,
        };
        state.gpr[1] = 7; // RCX
        assert_eq!(service_double(&state).unwrap(), 14);
    }

    #[test]
    fn service_double_wraps_on_overflow() {
        // u64::MAX * 2 wraps to u64::MAX - 1.
        let mut state = crate::cpu::State {
            gpr: [0; 16],
            rip: 0,
            rflags: 2,
        };
        state.gpr[1] = u64::MAX;
        assert_eq!(service_double(&state).unwrap(), u64::MAX.wrapping_mul(2));
    }
}
