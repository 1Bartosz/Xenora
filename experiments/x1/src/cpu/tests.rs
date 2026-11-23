// SPDX-License-Identifier: GPL-3.0-or-later
use super::{Fault, FaultKind, State, run};
use crate::fixture::{NONWRAP, OUTPUT, OUTPUT_BASE, WRAP, initial};

fn known_answer(code: &[u8], expected_trace: &[(u64, u64, u64)], expected_output: [u8; 8]) {
    let mut state = initial();
    let start = state.clone();
    let mut backing = [0xcc; 10];
    backing[1..9].copy_from_slice(&OUTPUT);
    let mut trace = Vec::new();

    let output = (&mut backing[1..9]).try_into().unwrap();
    assert_eq!(
        run(code, &mut state, OUTPUT_BASE, output, 16, |state| {
            trace.push(state.clone());
        }),
        Ok(())
    );
    assert_eq!(trace.len(), expected_trace.len());
    // Literal ISA positions CF, PF, AF, ZF, SF and OF.
    let arithmetic_mask = 0x8d5;
    for (actual, &(rip, rax, arithmetic_flags)) in trace.iter().zip(expected_trace) {
        let mut registers = start.gpr;
        registers[0] = rax;
        assert_eq!(actual.gpr, registers);
        assert_eq!(actual.rip, rip);
        assert_eq!(actual.rflags & arithmetic_mask, arithmetic_flags);
        assert_eq!(actual.rflags & !arithmetic_mask, 2);
    }
    assert_eq!(Some(&state), trace.last());
    assert_eq!(backing[1..9], expected_output);
    assert_eq!([backing[0], backing[9]], [0xcc, 0xcc]);
}

#[test]
fn wrap_falls_through() {
    known_answer(
        WRAP,
        &[
            (0x1005, 0xffff_ffff, 0x00),
            (0x1008, 0, 0x55),
            (0x100b, 0, 0x44),
            (0x100d, 0, 0x44),
            (0x1012, 42, 0x44),
            (0x1015, 42, 0x44),
        ],
        [0x2a, 0, 0, 0, 0, 0, 0, 0],
    );
}

#[test]
fn nonwrap_takes_branch() {
    known_answer(
        NONWRAP,
        &[
            (0x1005, 0xffff_fffe, 0x00),
            (0x1008, 0xffff_ffff, 0x84),
            (0x100b, 0xffff_ffff, 0x84),
            (0x1012, 0xffff_ffff, 0x84),
            (0x1015, 0xffff_ffff, 0x84),
        ],
        [0xff, 0xff, 0xff, 0xff, 0, 0, 0, 0],
    );
}

fn exact_fault(
    code: &[u8],
    budget: usize,
    rdi: u64,
    expected: Fault,
    retired: usize,
    rax: u64,
    rflags: u64,
) {
    let mut state = initial();
    state.gpr[7] = rdi;
    let mut output = OUTPUT;
    let mut trace = Vec::new();
    let result = run(
        code,
        &mut state,
        OUTPUT_BASE,
        &mut output,
        budget,
        |state| {
            trace.push(state.clone());
        },
    );
    let mut registers = initial().gpr;
    registers[0] = rax;
    registers[7] = rdi;
    assert_eq!(
        state,
        State {
            gpr: registers,
            rip: expected.rip,
            rflags
        },
    );
    assert_eq!(result, Err(expected));
    assert_eq!(trace.len(), retired);
    if let Some(before_fault) = trace.last() {
        assert_eq!(before_fault, &state);
    }
    assert_eq!(output, OUTPUT);
}

#[test]
fn unsupported_opcode() {
    exact_fault(
        &[0x90],
        16,
        OUTPUT_BASE,
        Fault {
            kind: FaultKind::Unsupported,
            rip: 0x1000,
        },
        0,
        0xaabb_ccdd_1234_5678,
        2,
    );
}

#[test]
fn truncated_immediate() {
    exact_fault(
        &[0xb8, 0xff],
        16,
        OUTPUT_BASE,
        Fault {
            kind: FaultKind::Truncated,
            rip: 0x1000,
        },
        0,
        0xaabb_ccdd_1234_5678,
        2,
    );
}

#[test]
fn branch_into_immediate() {
    let mut code = NONWRAP.to_vec();
    code[12] = 1;
    exact_fault(
        &code,
        16,
        OUTPUT_BASE,
        Fault {
            kind: FaultKind::BranchTarget,
            rip: 0x100b,
        },
        3,
        0xffff_ffff,
        0x86,
    );
}

#[test]
fn reduced_budget() {
    exact_fault(
        WRAP,
        2,
        OUTPUT_BASE,
        Fault {
            kind: FaultKind::Budget,
            rip: 0x1008,
        },
        2,
        0,
        0x57,
    );
}

#[test]
fn store_outside_output() {
    exact_fault(
        WRAP,
        16,
        OUTPUT_BASE + 8,
        Fault {
            kind: FaultKind::OutputRange,
            rip: 0x1012,
        },
        5,
        42,
        0x46,
    );
}

#[test]
fn store_crosses_output_boundary() {
    exact_fault(
        WRAP,
        16,
        OUTPUT_BASE + 1,
        Fault {
            kind: FaultKind::OutputRange,
            rip: 0x1012,
        },
        5,
        42,
        0x46,
    );
}

// ---------------------------------------------------------------------------
// New tests: SUB EAX,1  (83 E8 01)
// ---------------------------------------------------------------------------

/// `SUB EAX,1` with EAX=3: result=2, CF=0, PF=0 (one '1' bit), ZF=0, SF=0, AF=0, OF=0.
/// Independently derived: 3−1=2; parity(2)=0 (one set bit); no borrow; no overflow.
#[test]
fn sub_one_no_borrow() {
    // MOV EAX,3 | SUB EAX,1 | STORE [RDI]
    let code = [
        0xb8, 0x03, 0x00, 0x00, 0x00, // MOV EAX,3     [0..5]
        0x83, 0xe8, 0x01, // SUB EAX,1     [5..8]
        0x48, 0x89, 0x07, // STORE [RDI]   [8..11]
    ];
    // Independently expected trace (arithmetic_mask bits: CF PF AF ZF SF OF = 0x8d5):
    // MOV: rip=0x1005 rax=3   flags=0x00
    // SUB: rip=0x1008 rax=2   flags=0x00  (parity(2)=0, ZF=0, SF=0, CF=0)
    // STR: rip=0x100b rax=2   flags=0x00
    known_answer(
        &code,
        &[(0x1005, 3, 0x00), (0x1008, 2, 0x00), (0x100b, 2, 0x00)],
        [2, 0, 0, 0, 0, 0, 0, 0],
    );
}

/// `SUB EAX,1` with EAX=1: result=0, ZF=1, PF=1 (parity of 0 = zero bits = even).
/// Independently derived: 1−1=0; parity(0)=1; ZF=1; CF=0; SF=0.  flags=0x44.
#[test]
fn sub_one_reaches_zero() {
    // MOV EAX,1 | SUB EAX,1 | STORE [RDI]
    let code = [
        0xb8, 0x01, 0x00, 0x00, 0x00, // MOV EAX,1
        0x83, 0xe8, 0x01, // SUB EAX,1
        0x48, 0x89, 0x07, // STORE [RDI]
    ];
    // rax=0, flags=PF|ZF=0x44
    known_answer(
        &code,
        &[(0x1005, 1, 0x00), (0x1008, 0, 0x44), (0x100b, 0, 0x44)],
        [0, 0, 0, 0, 0, 0, 0, 0],
    );
}

/// `SUB EAX,1` with EAX=0: underflow borrow. result=0xffff_ffff, CF=1, SF=1,
/// PF=1 (parity of 0xff=8 ones=even), AF=1 (low-nibble borrow: 0−1 borrows).
/// Independently derived: 0−1 wraps to 0xffff_ffff; CF=1; SF=1; PF=1; AF=1; ZF=0; OF=0.
/// arithmetic flags = CF|PF|AF|SF = 0x01|0x04|0x10|0x80 = 0x95.
#[test]
fn sub_one_underflow() {
    exact_fault(
        // MOV EAX,0 | SUB EAX,1 | STORE [RDI] — budget=1 so only MOV retires.
        &[
            0xb8, 0x00, 0x00, 0x00, 0x00, // MOV EAX,0
            0x83, 0xe8, 0x01, // SUB EAX,1
            0x48, 0x89, 0x07, // STORE [RDI]
        ],
        1,
        OUTPUT_BASE,
        Fault {
            kind: FaultKind::Budget,
            rip: 0x1005, // stopped after MOV, before SUB
        },
        1,
        0, // rax after MOV EAX,0
        2, // rflags unchanged by MOV
    );
}

/// SUB underflow flag check: run full sequence to verify CF, PF, AF, SF set.
#[test]
fn sub_one_underflow_flags() {
    // MOV EAX,0 | SUB EAX,1 | STORE [RDI]
    // After SUB: rax=0xffff_ffff, flags=CF|PF|AF|SF=0x95
    let code = [
        0xb8, 0x00, 0x00, 0x00, 0x00, 0x83, 0xe8, 0x01, 0x48, 0x89, 0x07,
    ];
    known_answer(
        &code,
        &[
            (0x1005, 0, 0x00),
            (0x1008, 0xffff_ffff, 0x95),
            (0x100b, 0xffff_ffff, 0x95),
        ],
        [0xff, 0xff, 0xff, 0xff, 0, 0, 0, 0],
    );
}

// ---------------------------------------------------------------------------
// New tests: CMP EAX,imm8  (83 F8 imm)
// ---------------------------------------------------------------------------

/// `CMP EAX,imm8` with EAX==imm: result=0, ZF=1, PF=1. EAX unchanged.
/// Independently: 5−5=0, parity(0)=1, ZF=1, CF=0, SF=0, OF=0.  flags=0x44.
#[test]
fn cmp_imm8_equal() {
    // MOV EAX,5 | CMP EAX,5 | STORE [RDI]  — EAX still 5 after CMP
    let code = [
        0xb8, 0x05, 0x00, 0x00, 0x00, // MOV EAX,5
        0x83, 0xf8, 0x05, // CMP EAX,5
        0x48, 0x89, 0x07, // STORE [RDI]
    ];
    known_answer(
        &code,
        &[(0x1005, 5, 0x00), (0x1008, 5, 0x44), (0x100b, 5, 0x44)],
        [5, 0, 0, 0, 0, 0, 0, 0],
    );
}

/// `CMP EAX,imm8` with EAX > imm (unsigned): ZF=0, CF=0, SF=0.
/// Independently: 3−2=1, parity(1)=0, ZF=0, CF=0, SF=0.  flags=0x00.
#[test]
fn cmp_imm8_greater() {
    let code = [
        0xb8, 0x03, 0x00, 0x00, 0x00, // MOV EAX,3
        0x83, 0xf8, 0x02, // CMP EAX,2
        0x48, 0x89, 0x07, // STORE [RDI]
    ];
    known_answer(
        &code,
        &[(0x1005, 3, 0x00), (0x1008, 3, 0x00), (0x100b, 3, 0x00)],
        [3, 0, 0, 0, 0, 0, 0, 0],
    );
}

/// Truncated `83 F8` (CMP with no immediate): must fault as Truncated before retiring.
#[test]
fn cmp_imm8_truncated() {
    exact_fault(
        &[0x83, 0xf8],
        16,
        OUTPUT_BASE,
        Fault {
            kind: FaultKind::Truncated,
            rip: 0x1000,
        },
        0,
        0xaabb_ccdd_1234_5678,
        2,
    );
}

/// Truncated `83 E8` (SUB with no immediate): must fault as Truncated.
#[test]
fn sub_one_truncated() {
    exact_fault(
        &[0x83, 0xe8],
        16,
        OUTPUT_BASE,
        Fault {
            kind: FaultKind::Truncated,
            rip: 0x1000,
        },
        0,
        0xaabb_ccdd_1234_5678,
        2,
    );
}

// ---------------------------------------------------------------------------
// New tests: JE rel8 (74 disp) — jump if ZF=1
// ---------------------------------------------------------------------------

/// `JE` taken: CMP EAX,EAX (equal) sets ZF, JE jumps over a MOV, STORE gets EAX=5.
/// Independently: MOV 5; CMP 5,5 → ZF=1; JE +5 → skip MOV EAX,0xff; STORE → output=5.
/// Code layout: [0]MOV5 [5]CMP5,5 [8]JE+5 [10]MOV0xff [15]STORE — 18 bytes.
#[test]
fn je_taken_skips_instruction() {
    let code = [
        0xb8, 0x05, 0x00, 0x00, 0x00, // MOV EAX,5     [0..5]
        0x83, 0xf8, 0x05, // CMP EAX,5     [5..8] → ZF=1 flags=0x44
        0x74, 0x05, // JE  +5        [8..10] → target=10+5=15
        0xb8, 0xff, 0x00, 0x00, 0x00, // MOV EAX,255   [10..15] skipped
        0x48, 0x89, 0x07, // STORE [RDI]   [15..18]
    ];
    // Trace: MOV(rip=1005,rax=5,f=0), CMP(rip=1008,rax=5,f=0x44),
    //        JE-taken(rip=100f,rax=5,f=0x44), STORE(rip=1012,rax=5,f=0x44).
    // Note: rip=0x100f=0x1000+15=15, rip after STORE=0x1000+18=0x1012.
    known_answer(
        &code,
        &[
            (0x1005, 5, 0x00),
            (0x1008, 5, 0x44),
            (0x100f, 5, 0x44),
            (0x1012, 5, 0x44),
        ],
        [5, 0, 0, 0, 0, 0, 0, 0],
    );
}

/// `JE` not taken: CMP EAX,imm (unequal) leaves ZF=0, JE falls through to MOV.
/// Independently: MOV 5; CMP 5,3 → ZF=0 flags=0x00; JE +5 falls through;
/// MOV 42; STORE → output=42.
#[test]
fn je_not_taken_falls_through() {
    let code = [
        0xb8, 0x05, 0x00, 0x00, 0x00, // MOV EAX,5     [0..5]
        0x83, 0xf8, 0x03, // CMP EAX,3     [5..8] → ZF=0 flags=0x00
        0x74, 0x05, // JE  +5        [8..10] → not taken
        0xb8, 0x2a, 0x00, 0x00, 0x00, // MOV EAX,42    [10..15]
        0x48, 0x89, 0x07, // STORE [RDI]   [15..18]
    ];
    // Trace: MOV5, CMP5,3(f=0), JE-fall(rip=0x100a), MOV42(rip=0x100f), STORE(rip=0x1012).
    known_answer(
        &code,
        &[
            (0x1005, 5, 0x00),
            (0x1008, 5, 0x00),
            (0x100a, 5, 0x00),
            (0x100f, 42, 0x00),
            (0x1012, 42, 0x00),
        ],
        [42, 0, 0, 0, 0, 0, 0, 0],
    );
}

/// JE into the middle of an immediate (target=12, which is inside a 5-byte MOV) → BranchTarget.
/// Layout: [0]MOV1 [5]CMP1,1 [8]JE+2 (target=12) [10]MOV255 [15]STORE.
/// Independently: JE next=10, disp=2, target=12; scan 0→5→8→10→15>12; boundary mismatch → fault.
#[test]
fn je_into_immediate_faults() {
    exact_fault(
        &[
            0xb8, 0x01, 0x00, 0x00, 0x00, // MOV EAX,1  [0..5]
            0x83, 0xf8, 0x01, // CMP EAX,1  [5..8] → ZF=1
            0x74, 0x02, // JE +2      [8..10] target=12 (into MOV immediate)
            0xb8, 0xff, 0x00, 0x00, 0x00, // MOV EAX,255[10..15]
            0x48, 0x89, 0x07, // STORE      [15..18]
        ],
        16,
        OUTPUT_BASE,
        Fault {
            kind: FaultKind::BranchTarget,
            rip: 0x1008, // the JE instruction's rip
        },
        2,        // MOV and CMP retired before JE faults
        1,        // rax after MOV EAX,1
        2 | 0x44, // rflags: bit1 | CMP ZF|PF
    );
}

/// Truncated `74` (JE with no displacement): Truncated fault.
#[test]
fn je_truncated() {
    exact_fault(
        &[0x74],
        16,
        OUTPUT_BASE,
        Fault {
            kind: FaultKind::Truncated,
            rip: 0x1000,
        },
        0,
        0xaabb_ccdd_1234_5678,
        2,
    );
}

// ---------------------------------------------------------------------------
// New tests: backward branch + SUB countdown loop
// ---------------------------------------------------------------------------

/// End-to-end countdown loop using SUB+CMP+JNE with a backward branch.
/// Guest program counts down from 3 to 0, then stores the final EAX=0.
///
/// Independently derived expected state at each retirement (11 steps):
/// Code layout (16 bytes):
///   [0] B8 03 00 00 00  MOV EAX,3
///   [5] 83 E8 01        SUB EAX,1
///   [8] 83 F8 00        CMP EAX,0
///  [11] 75 F8           JNE -8 (target=13+(-8)=5, instruction boundary)
///  [13] 48 89 07        STORE [RDI]
///
/// Budget=16 ≥ 11 steps.  Output: EAX=0 as little-endian u64 = [0;8].
#[test]
fn countdown_loop_backward_branch() {
    let code = [
        0xb8, 0x03, 0x00, 0x00, 0x00, // MOV EAX,3   [0..5]
        0x83, 0xe8, 0x01, // SUB EAX,1   [5..8]
        0x83, 0xf8, 0x00, // CMP EAX,0   [8..11]
        0x75, 0xf8, // JNE -8      [11..13] → target=5
        0x48, 0x89, 0x07, // STORE [RDI] [13..16]
    ];
    // 11 retirements.  Independently derived arithmetic_flags (arithmetic_mask=0x8d5):
    // MOV: rax=3, flags=0x00
    // SUB 3→2: parity(2)=0, ZF=0, CF=0, SF=0 → flags=0x00
    // CMP 2,0: 2-0=2, same as above → flags=0x00
    // JNE taken (ZF=0): rax=2, flags=0x00
    // SUB 2→1: parity(1)=0, ZF=0, CF=0, SF=0 → flags=0x00
    // CMP 1,0: flags=0x00
    // JNE taken: rax=1, flags=0x00
    // SUB 1→0: parity(0)=1→PF, ZF=1 → flags=0x44
    // CMP 0,0: 0-0=0, PF=1, ZF=1 → flags=0x44
    // JNE falls (ZF=1): rax=0, flags=0x44
    // STORE: rax=0, flags=0x44
    known_answer(
        &code,
        &[
            (0x1005, 3, 0x00), // MOV
            (0x1008, 2, 0x00), // SUB 3→2
            (0x100b, 2, 0x00), // CMP 2,0
            (0x1005, 2, 0x00), // JNE taken
            (0x1008, 1, 0x00), // SUB 2→1
            (0x100b, 1, 0x00), // CMP 1,0
            (0x1005, 1, 0x00), // JNE taken
            (0x1008, 0, 0x44), // SUB 1→0: PF|ZF
            (0x100b, 0, 0x44), // CMP 0,0: PF|ZF
            (0x100d, 0, 0x44), // JNE falls through
            (0x1010, 0, 0x44), // STORE
        ],
        [0, 0, 0, 0, 0, 0, 0, 0],
    );
}

/// Budget exhaustion in a backward-branching loop: loop body gets stuck repeating.
/// Budget=4 retires MOV+SUB+CMP+JNE-taken (rip back to SUB), then hits limit.
#[test]
fn countdown_loop_budget_exhaustion() {
    let code = [
        0xb8, 0x03, 0x00, 0x00, 0x00, // MOV EAX,3   [0..5]
        0x83, 0xe8, 0x01, // SUB EAX,1   [5..8]
        0x83, 0xf8, 0x00, // CMP EAX,0   [8..11]
        0x75, 0xf8, // JNE -8      [11..13]
        0x48, 0x89, 0x07, // STORE [RDI] [13..16]
    ];
    // After 4 retirements: MOV, SUB (rax=2), CMP (rax=2), JNE-taken (rip=1005=SUB).
    // Budget=4 exhausted; rip=0x1005.
    exact_fault(
        &code,
        4,
        OUTPUT_BASE,
        Fault {
            kind: FaultKind::Budget,
            rip: 0x1005,
        },
        4,
        2, // rax after SUB 3→2 (JNE taken jumps back but rax was already 2)
        2, // rflags: bit1 | CMP 2,0 flags=0x00
    );
}
