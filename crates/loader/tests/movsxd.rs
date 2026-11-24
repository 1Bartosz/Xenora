// SPDX-License-Identifier: GPL-3.0-or-later

//! Interpreter known-answer tests for the x86-64 MOVSX/MOVSXD opcode (0x63),
//! see issue #117.

use xenora_loader::cpu::{CpuState, ExitReason, Interpreter, Register};
use xenora_loader::memory::{GuestMemory, MemoryPermissions};

const ENTRY: u64 = 0x1000;

/// Run `code` through the interpreter with a one-instruction budget.
fn run_code(code: &[u8], cpu: CpuState) -> xenora_loader::cpu::ExecutionResult {
    let mut memory = GuestMemory::new();
    memory
        .map_region_with_data(ENTRY, code.len(), MemoryPermissions::READ_EXECUTE, code)
        .unwrap();
    Interpreter::new(cpu, &mut memory, 1).run()
}

#[test]
fn movsxd_sign_extends_negative_32_to_64() {
    // 48 63 D8 = MOVSXD rbx, eax (REX.W; src=rax, dst=rbx; modrm 0xd8).
    let mut cpu = CpuState::new(ENTRY, 0);
    cpu.gpr[Register::Rax as usize] = 0xFFFF_FFF1;
    let result = run_code(&[0x48, 0x63, 0xD8], cpu);
    assert_eq!(result.exit_reason, ExitReason::StepLimitExceeded);
    assert_eq!(result.instructions_executed, 1);
    assert_eq!(
        result.final_state.gpr[Register::Rbx as usize],
        0xFFFF_FFFF_FFFF_FFF1
    );
}

#[test]
fn movsxd_sign_extends_positive_32_to_64() {
    let mut cpu = CpuState::new(ENTRY, 0);
    cpu.gpr[Register::Rax as usize] = 0x7FFF_FFF1;
    let result = run_code(&[0x48, 0x63, 0xD8], cpu);
    assert_eq!(result.instructions_executed, 1);
    assert_eq!(result.final_state.gpr[Register::Rbx as usize], 0x7FFF_FFF1);
}

#[test]
fn movsxd_without_rex_w_zeroes_upper_bits() {
    // 63 D8 = MOVSX ebx, eax: writing a 32-bit register zeroes the upper half.
    let mut cpu = CpuState::new(ENTRY, 0);
    cpu.gpr[Register::Rax as usize] = 0xFFFF_FFF1;
    cpu.gpr[Register::Rbx as usize] = 0xFFFF_FFFF_FFFF_FFFF;
    let result = run_code(&[0x63, 0xD8], cpu);
    assert_eq!(result.instructions_executed, 1);
    assert_eq!(result.final_state.gpr[Register::Rbx as usize], 0xFFFF_FFF1);
}

#[test]
fn movsxd_uses_rex_r_and_rex_b() {
    // 4D 63 C1 = MOVSXD r8, r9 (REX.W | REX.R | REX.B; modrm 0xc1).
    let mut cpu = CpuState::new(ENTRY, 0);
    cpu.gpr[Register::R9 as usize] = 0x8000_0001;
    let result = run_code(&[0x4D, 0x63, 0xC1], cpu);
    assert_eq!(result.instructions_executed, 1);
    assert_eq!(
        result.final_state.gpr[Register::R8 as usize],
        0xFFFF_FFFF_8000_0001
    );
    assert_eq!(result.final_state.gpr[Register::R9 as usize], 0x8000_0001);
}

#[test]
fn movsxd_rejects_non_register_addressing() {
    // 48 63 00: 0x63 is MOVSXD only in register-to-register form; with modrm
    // 0x00 (mode 0) this is an illegal instruction for the interpreter.
    let result = run_code(&[0x48, 0x63, 0x00], CpuState::new(ENTRY, 0));
    assert!(matches!(
        result.exit_reason,
        ExitReason::IllegalInstruction(_)
    ));
    assert_eq!(result.instructions_executed, 0);
}
