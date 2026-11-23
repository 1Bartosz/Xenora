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
