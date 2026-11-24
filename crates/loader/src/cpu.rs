// SPDX-License-Identifier: GPL-3.0-or-later

//! Minimal x86-64 guest CPU register state and bounded instruction interpreter.
//!
//! Executes real x86-64 opcodes in software against isolated `GuestMemory`.
//! Never executes guest x86-64 code natively on host ARM64.

use crate::memory::{GuestMemory, MemoryError};
use crate::x86_decode::MAX_INSTRUCTION_LEN;
use std::fmt;

/// 64-bit General Purpose Register identifiers in x86-64.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Register {
    Rax = 0,
    Rcx = 1,
    Rdx = 2,
    Rbx = 3,
    Rsp = 4,
    Rbp = 5,
    Rsi = 6,
    Rdi = 7,
    R8 = 8,
    R9 = 9,
    R10 = 10,
    R11 = 11,
    R12 = 12,
    R13 = 13,
    R14 = 14,
    R15 = 15,
}

impl Register {
    pub fn from_index(index: u8) -> Self {
        match index & 0xF {
            0 => Self::Rax,
            1 => Self::Rcx,
            2 => Self::Rdx,
            3 => Self::Rbx,
            4 => Self::Rsp,
            5 => Self::Rbp,
            6 => Self::Rsi,
            7 => Self::Rdi,
            8 => Self::R8,
            9 => Self::R9,
            10 => Self::R10,
            11 => Self::R11,
            12 => Self::R12,
            13 => Self::R13,
            14 => Self::R14,
            15 => Self::R15,
            _ => unreachable!(),
        }
    }
}

/// Instruction prefixes decoded ahead of an opcode.
///
/// The interpreter decodes these once per instruction and passes them to the
/// helpers that need them, so prefix state travels as one typed value instead
/// of a list of independent booleans.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Prefixes {
    /// REX.W: 64-bit operand size.
    rex_w: bool,
    /// REX.R: extends the ModRM `reg` field to R8-R15.
    rex_r: bool,
    /// REX.X: extends the SIB `index` field to R8-R15.
    rex_x: bool,
    /// REX.B: extends the ModRM `rm` field to R8-R15.
    rex_b: bool,
    /// The GS segment override prefix (0x65) is present.
    seg_gs: bool,
    /// The FS segment override prefix (0x64) is present.
    seg_fs: bool,
}

/// RFLAGS status bits tracked during guest execution.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RFlags {
    pub cf: bool, // Carry flag
    pub zf: bool, // Zero flag
    pub sf: bool, // Sign flag
    pub of: bool, // Overflow flag
}

/// x86-64 CPU state. Native DBT accesses the general registers and RIP by offset.
#[repr(C)]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CpuState {
    pub gpr: [u64; 16],
    pub rip: u64,
    pub flags: RFlags,
    pub gs_base: u64,
    pub fs_base: u64,
}

impl CpuState {
    pub fn new(entry_point: u64, stack_pointer: u64) -> Self {
        let mut gpr = [0u64; 16];
        gpr[Register::Rsp as usize] = stack_pointer;
        Self {
            gpr,
            rip: entry_point,
            flags: RFlags::default(),
            gs_base: 0,
            fs_base: 0,
        }
    }

    pub fn get(&self, reg: Register) -> u64 {
        self.gpr[reg as usize]
    }

    pub fn set(&mut self, reg: Register, val: u64) {
        self.gpr[reg as usize] = val;
    }
}

/// Termination reason for guest execution.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExitReason {
    Halt,
    Syscall(u64),
    StepLimitExceeded,
    JitCacheExhausted,
    MemoryFault(MemoryError),
    IllegalInstruction(u8),
    StackUnderflow,
}

impl fmt::Display for ExitReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Halt => write!(f, "guest executed HLT"),
            Self::Syscall(nr) => write!(f, "guest executed SYSCALL ({nr})"),
            Self::StepLimitExceeded => write!(f, "execution step budget exhausted"),
            Self::JitCacheExhausted => write!(f, "native JIT code cache exhausted"),
            Self::MemoryFault(e) => write!(f, "memory fault: {e}"),
            Self::IllegalInstruction(op) => write!(f, "illegal or unsupported opcode: 0x{op:02x}"),
            Self::StackUnderflow => write!(f, "stack underflow"),
        }
    }
}

impl std::error::Error for ExitReason {}

#[cfg(test)]
mod layout_tests {
    use super::{CpuState, RFlags, Register};

    /// The native translator addresses guest registers and RIP by raw byte
    /// offset. Keep these assertions next to the struct so a layout change
    /// cannot silently corrupt translated blocks.
    #[test]
    fn native_translator_offsets_match_repr_c_layout() {
        assert_eq!(std::mem::offset_of!(CpuState, gpr), 0);
        assert_eq!(std::mem::offset_of!(CpuState, rip), 16 * 8);
        assert_eq!(std::mem::offset_of!(CpuState, flags), 16 * 8 + 8);
        assert_eq!(std::mem::offset_of!(CpuState, gs_base), 16 * 8 + 8 + 8);
        assert_eq!(std::mem::offset_of!(CpuState, fs_base), 16 * 8 + 8 + 8 + 8);
        assert_eq!(std::mem::size_of::<RFlags>(), 4);
        assert_eq!(std::mem::size_of::<CpuState>(), 16 * 8 + 8 + 8 + 8 + 8);

        let cpu = CpuState::default();
        let base = std::ptr::addr_of!(cpu) as usize;
        for reg in [Register::Rax, Register::Rsp, Register::R15] {
            assert_eq!(
                base + (reg as usize) * 8,
                std::ptr::addr_of!(cpu.gpr[reg as usize]) as usize
            );
        }
        assert_eq!(base + 16 * 8, std::ptr::addr_of!(cpu.rip) as usize);
        assert_eq!(base + 16 * 8 + 8, std::ptr::addr_of!(cpu.flags) as usize);
    }
}

#[cfg(test)]
mod prefix_tests {
    use super::{CpuState, ExitReason, Interpreter, Register};
    use crate::memory::{GuestMemory, MemoryPermissions};

    const CODE_BASE: u64 = 0x1000;
    const CODE_SIZE: usize = 0x1000;
    const STACK_BASE: u64 = 0x8000_0000;
    const STACK_SIZE: usize = 0x10000;
    const FS_BASE: u64 = 0x9000;
    const GS_BASE: u64 = 0xA000;

    /// A guest memory image with executable code, a stack and distinct FS and
    /// GS bases.
    fn memory_with_code(code: &[u8]) -> GuestMemory {
        let mut memory = GuestMemory::new();
        memory
            .map_region_with_data(
                CODE_BASE,
                CODE_SIZE,
                MemoryPermissions::READ_WRITE_EXECUTE,
                code,
            )
            .expect("code region");
        memory
            .map_region(STACK_BASE, STACK_SIZE, MemoryPermissions::READ_WRITE)
            .expect("stack region");
        // The segment bases are distinct so a swapped prefix is visible.
        memory
            .map_region(FS_BASE, 0x1000, MemoryPermissions::READ_WRITE)
            .expect("fs region");
        memory
            .map_region(GS_BASE, 0x1000, MemoryPermissions::READ_WRITE)
            .expect("gs region");
        memory
    }

    /// Run `code` on the interpreter and return the final state and exit reason.
    fn run(code: &[u8]) -> (CpuState, ExitReason) {
        let mut memory = memory_with_code(code);
        let cpu = CpuState::new(CODE_BASE, STACK_BASE + 0x8000);
        let mut interp = Interpreter::new(cpu, &mut memory, 64);
        let result = interp.run();
        (result.final_state, result.exit_reason)
    }

    /// `mov r8, imm64`: 49 B8 <imm64>.
    fn mov_r8_imm64(value: u64) -> Vec<u8> {
        let mut code = vec![0x49, 0xB8];
        code.extend_from_slice(&value.to_le_bytes());
        code
    }

    /// `mov rbx, imm64`: 48 BB <imm64>.
    fn mov_rbx_imm64(value: u64) -> Vec<u8> {
        let mut code = vec![0x48, 0xBB];
        code.extend_from_slice(&value.to_le_bytes());
        code
    }

    /// `mov rax, imm64`: 48 B8 <imm64>.
    fn mov_rax_imm64(value: u64) -> Vec<u8> {
        let mut code = vec![0x48, 0xB8];
        code.extend_from_slice(&value.to_le_bytes());
        code
    }

    /// REX.B extends the ModRM `rm` field, so `rm=0` selects r8.
    ///
    /// Encoding 49 89 D8 is `mov r8, rbx`.
    #[test]
    fn rex_b_extends_the_rm_field_to_r8() {
        let mut code = mov_rbx_imm64(0x1122_3344_5566_7788);
        code.extend_from_slice(&[0x49, 0x89, 0xD8]); // mov r8, rbx
        code.push(0xF4); // hlt

        let (state, reason) = run(&code);
        assert_eq!(reason, ExitReason::Halt);
        assert_eq!(state.get(Register::Rbx), 0x1122_3344_5566_7788);
        assert_eq!(state.get(Register::R8), 0x1122_3344_5566_7788);
        // No prefix in this encoding selects r9.
        assert_eq!(state.get(Register::R9), 0);
    }

    /// REX.R extends the ModRM `reg` field, so `reg=0` selects r8.
    ///
    /// Encoding 4C 89 C3 is `mov rbx, r8`.
    #[test]
    fn rex_r_extends_the_reg_field_to_r8() {
        let mut code = mov_r8_imm64(0x1122_3344_5566_7788);
        code.extend_from_slice(&[0x4C, 0x89, 0xC3]); // mov rbx, r8
        code.push(0xF4); // hlt

        let (state, reason) = run(&code);
        assert_eq!(reason, ExitReason::Halt);
        assert_eq!(state.get(Register::R8), 0x1122_3344_5566_7788);
        assert_eq!(state.get(Register::Rbx), 0x1122_3344_5566_7788);
        // No prefix in this encoding selects r9.
        assert_eq!(state.get(Register::R9), 0);
    }

    /// REX.R and REX.B extend independent fields, so the destination r9 and
    /// the source r12 are chosen by different prefix bits.
    ///
    /// Encoding 4D 89 E1 is `mov r9, r12`.
    #[test]
    fn rex_r_and_rex_b_extend_independent_fields() {
        let mut code = mov_r8_imm64(0x1122_3344_5566_7788);
        code.extend_from_slice(&[0x4D, 0x89, 0xE1]); // mov r9, r12
        code.push(0xF4); // hlt

        let (state, reason) = run(&code);
        assert_eq!(reason, ExitReason::Halt);
        // r9 receives r12, which is zero here, and not the value in r8.
        assert_eq!(state.get(Register::R9), 0);
        assert_eq!(state.get(Register::R12), 0);
        assert_eq!(state.get(Register::R8), 0x1122_3344_5566_7788);
    }

    /// Without REX.W the move is 32-bit wide, so the destination keeps only the
    /// low half of the source and its upper half is cleared.
    ///
    /// Encoding 89 D8 is `mov eax, ebx`.
    #[test]
    fn a_move_without_rex_w_is_32_bit_wide() {
        let mut code = mov_rbx_imm64(0x1122_3344_5566_7788);
        code.extend_from_slice(&[0x89, 0xD8]); // mov eax, ebx
        code.push(0xF4); // hlt

        let (state, reason) = run(&code);
        assert_eq!(reason, ExitReason::Halt);
        // Only the low 32 bits of ebx survive, and rax's upper half is cleared.
        assert_eq!(state.get(Register::Rax), 0x5566_7788);
        assert_eq!(state.get(Register::Rbx), 0x1122_3344_5566_7788);
    }

    /// The FS segment override adds the FS base to the effective address.
    ///
    /// Encoding 64 48 89 04 25 08 00 00 00 is `mov fs:[0x8], rax`.
    #[test]
    fn the_fs_prefix_adds_the_fs_base() {
        let mut code = mov_rax_imm64(0x1122_3344_5566_7788);
        code.extend_from_slice(&[0x64, 0x48, 0x89, 0x04, 0x25, 0x08, 0x00, 0x00, 0x00]);
        code.push(0xF4); // hlt

        let mut memory = memory_with_code(&code);
        let mut cpu = CpuState::new(CODE_BASE, STACK_BASE + 0x8000);
        cpu.fs_base = FS_BASE;
        cpu.gs_base = GS_BASE;
        let mut interp = Interpreter::new(cpu, &mut memory, 64);
        let result = interp.run();
        assert_eq!(result.exit_reason, ExitReason::Halt);
        assert_eq!(
            memory.read_u64(FS_BASE + 0x8).expect("fs read"),
            0x1122_3344_5566_7788
        );
        // The GS base is untouched: the GS-prefixed store did not happen.
        assert_eq!(memory.read_u64(GS_BASE + 0x8).expect("gs read"), 0);
    }
}

/// Result returned after running guest execution.
#[derive(Debug)]
pub struct ExecutionResult {
    pub instructions_executed: usize,
    pub final_state: CpuState,
    pub exit_reason: ExitReason,
}

/// Bounded software interpreter for x86-64 instructions.
pub struct Interpreter<'a> {
    pub cpu: CpuState,
    pub memory: &'a mut GuestMemory,
    pub max_instructions: usize,
    pub instructions_executed: usize,
}

impl<'a> Interpreter<'a> {
    pub fn new(cpu: CpuState, memory: &'a mut GuestMemory, max_instructions: usize) -> Self {
        Self {
            cpu,
            memory,
            max_instructions,
            instructions_executed: 0,
        }
    }

    /// Run interpreter until halt, error, or instruction budget limit.
    pub fn run(&mut self) -> ExecutionResult {
        while self.instructions_executed < self.max_instructions {
            match self.step() {
                Ok(None) => {
                    self.instructions_executed += 1;
                }
                Ok(Some(reason)) => {
                    self.instructions_executed += 1;
                    return ExecutionResult {
                        instructions_executed: self.instructions_executed,
                        final_state: self.cpu.clone(),
                        exit_reason: reason,
                    };
                }
                Err(reason) => {
                    return ExecutionResult {
                        instructions_executed: self.instructions_executed,
                        final_state: self.cpu.clone(),
                        exit_reason: reason,
                    };
                }
            }
        }

        ExecutionResult {
            instructions_executed: self.instructions_executed,
            final_state: self.cpu.clone(),
            exit_reason: ExitReason::StepLimitExceeded,
        }
    }

    fn fetch_u8(&mut self) -> Result<u8, ExitReason> {
        let b = self
            .memory
            .fetch_instruction_byte(self.cpu.rip)
            .map_err(ExitReason::MemoryFault)?;
        self.cpu.rip += 1;
        Ok(b)
    }

    fn fetch_u32(&mut self) -> Result<u32, ExitReason> {
        let mut bytes = [0u8; 4];
        for b in &mut bytes {
            *b = self.fetch_u8()?;
        }
        Ok(u32::from_le_bytes(bytes))
    }

    fn fetch_u64(&mut self) -> Result<u64, ExitReason> {
        let mut bytes = [0u8; 8];
        for b in &mut bytes {
            *b = self.fetch_u8()?;
        }
        Ok(u64::from_le_bytes(bytes))
    }

    fn fetch_i8(&mut self) -> Result<i8, ExitReason> {
        Ok(self.fetch_u8()? as i8)
    }

    fn fetch_i32(&mut self) -> Result<i32, ExitReason> {
        Ok(self.fetch_u32()? as i32)
    }

    /// Single-step execution. Returns Ok(Some(ExitReason)) on clean exit (HLT/SYSCALL).
    pub fn step(&mut self) -> Result<Option<ExitReason>, ExitReason> {
        let start = self.cpu.rip;
        let result = self.execute_instruction();
        if result.is_err() {
            self.cpu.rip = start;
        }
        result
    }

    fn execute_instruction(&mut self) -> Result<Option<ExitReason>, ExitReason> {
        let mut prefixes = Prefixes::default();

        let mut byte = self.fetch_u8()?;
        let mut insn_len = 1usize; // first opcode byte already fetched

        // Handle legacy segment prefixes: 0x65 (GS), 0x64 (FS). The run is
        // bounded by the architectural instruction-length limit so an
        // overlong prefix run is reported as an illegal instruction.
        while byte == 0x65 || byte == 0x64 {
            if insn_len >= MAX_INSTRUCTION_LEN {
                return Err(ExitReason::IllegalInstruction(byte));
            }
            if byte == 0x65 {
                prefixes.seg_gs = true;
            } else {
                prefixes.seg_fs = true;
            }
            byte = self.fetch_u8()?;
            insn_len += 1;
        }

        // Handle REX prefix (0x40 - 0x4F)
        if (byte & 0xF0) == 0x40 {
            if insn_len >= MAX_INSTRUCTION_LEN {
                return Err(ExitReason::IllegalInstruction(byte));
            }
            prefixes.rex_w = (byte & 0x08) != 0;
            prefixes.rex_r = (byte & 0x04) != 0;
            prefixes.rex_x = (byte & 0x02) != 0;
            prefixes.rex_b = (byte & 0x01) != 0;
            byte = self.fetch_u8()?;
        }

        match byte {
            // HLT: Halt execution
            0xF4 => Ok(Some(ExitReason::Halt)),

            0x0F => match self.fetch_u8()? {
                0x05 => Ok(Some(ExitReason::Syscall(self.cpu.get(Register::Rax)))),
                op @ 0xC8..=0xCF => {
                    let reg = Register::from_index((op & 7) | (u8::from(prefixes.rex_b) << 3));
                    let value = self.cpu.get(reg);
                    let reversed = if prefixes.rex_w {
                        value.swap_bytes()
                    } else {
                        u64::from((value as u32).swap_bytes())
                    };
                    self.cpu.set(reg, reversed);
                    Ok(None)
                }
                op => Err(ExitReason::IllegalInstruction(op)),
            },

            // MOV r32, imm32 (0xB8 + reg) or MOV r64, imm64 with REX.W
            0xB8..=0xBF => {
                let reg_idx = (byte - 0xB8) + if prefixes.rex_b { 8 } else { 0 };
                let reg = Register::from_index(reg_idx);
                if prefixes.rex_w {
                    let imm = self.fetch_u64()?;
                    self.cpu.set(reg, imm);
                } else {
                    let imm = self.fetch_u32()? as u64; // Zero-extended in x86-64
                    self.cpu.set(reg, imm);
                }
                Ok(None)
            }

            // XOR r/m, reg (0x31) or XOR reg, r/m (0x33)
            0x31 | 0x33 => {
                let modrm = self.fetch_u8()?;
                let (reg, rm) = self.decode_modrm_reg_reg(modrm, &prefixes)?;
                let (src, dst) = if byte == 0x31 { (reg, rm) } else { (rm, reg) };
                let val1 = self.cpu.get(dst);
                let val2 = self.cpu.get(src);
                let res = if prefixes.rex_w {
                    val1 ^ val2
                } else {
                    (val1 as u32 ^ val2 as u32) as u64
                };
                self.cpu.set(dst, res);
                self.update_flags_logic(res, prefixes.rex_w);
                Ok(None)
            }

            // ADD r/m, reg (0x01) or ADD reg, r/m (0x03)
            0x01 | 0x03 => {
                let modrm = self.fetch_u8()?;
                let (reg, rm) = self.decode_modrm_reg_reg(modrm, &prefixes)?;
                let (src, dst) = if byte == 0x01 { (reg, rm) } else { (rm, reg) };
                let val1 = self.cpu.get(dst);
                let val2 = self.cpu.get(src);
                let (res, flags) = if prefixes.rex_w {
                    add64(val1, val2)
                } else {
                    let (r, f) = add32(val1 as u32, val2 as u32);
                    (r as u64, f)
                };
                self.cpu.set(dst, res);
                self.cpu.flags = flags;
                Ok(None)
            }

            // SUB r/m, reg (0x29) or SUB reg, r/m (0x2B)
            0x29 | 0x2B => {
                let modrm = self.fetch_u8()?;
                let (reg, rm) = self.decode_modrm_reg_reg(modrm, &prefixes)?;
                let (src, dst) = if byte == 0x29 { (reg, rm) } else { (rm, reg) };
                let val1 = self.cpu.get(dst);
                let val2 = self.cpu.get(src);
                let (res, flags) = if prefixes.rex_w {
                    sub64(val1, val2)
                } else {
                    let (r, f) = sub32(val1 as u32, val2 as u32);
                    (r as u64, f)
                };
                self.cpu.set(dst, res);
                self.cpu.flags = flags;
                Ok(None)
            }

            // CMP reg, r/m (0x3B) or CMP r/m, reg (0x39)
            0x39 | 0x3B => {
                let modrm = self.fetch_u8()?;
                let (reg, rm) = self.decode_modrm_reg_reg(modrm, &prefixes)?;
                let (v1, v2) = if byte == 0x39 {
                    (self.cpu.get(rm), self.cpu.get(reg))
                } else {
                    (self.cpu.get(reg), self.cpu.get(rm))
                };
                let (_, flags) = if prefixes.rex_w {
                    sub64(v1, v2)
                } else {
                    let (r, f) = sub32(v1 as u32, v2 as u32);
                    (r as u64, f)
                };
                self.cpu.flags = flags;
                Ok(None)
            }

            // MOVSX r/m, r (0x63): sign-extend the source into the
            // destination register. With REX.W this is MOVSXD r64, r/m32.
            0x63 => {
                let modrm = self.fetch_u8()?;
                if (modrm >> 6) != 3 {
                    return Err(ExitReason::IllegalInstruction(byte));
                }
                let src_idx = (modrm & 7) + if prefixes.rex_b { 8 } else { 0 };
                let dst_idx = ((modrm >> 3) & 7) + if prefixes.rex_r { 8 } else { 0 };
                let src32 = self.cpu.get(Register::from_index(src_idx)) as u32;
                let dst = Register::from_index(dst_idx);
                if prefixes.rex_w {
                    // MOVSXD r64, r/m32: sign-extend the 32-bit source into 64 bits.
                    self.cpu.set(dst, src32 as i32 as u64);
                } else {
                    // MOVSX r32, r/m32: writing a 32-bit register zeroes the upper half.
                    self.cpu.set(dst, u64::from(src32 as i32 as u32));
                }
                Ok(None)
            }

            // Immediate group 1: 0x83 /op reg, imm8
            0x83 => {
                let modrm = self.fetch_u8()?;
                let op = (modrm >> 3) & 7;
                let (reg, rm) = self.decode_modrm_reg_reg(modrm, &prefixes)?;
                let imm8 = self.fetch_i8()? as i64;
                let imm = imm8 as u64;

                let val = self.cpu.get(rm);
                match op {
                    0 => {
                        // ADD
                        let (res, flags) = if prefixes.rex_w {
                            add64(val, imm)
                        } else {
                            let (r, f) = add32(val as u32, imm as u32);
                            (r as u64, f)
                        };
                        self.cpu.set(rm, res);
                        self.cpu.flags = flags;
                    }
                    5 => {
                        // SUB
                        let (res, flags) = if prefixes.rex_w {
                            sub64(val, imm)
                        } else {
                            let (r, f) = sub32(val as u32, imm as u32);
                            (r as u64, f)
                        };
                        self.cpu.set(rm, res);
                        self.cpu.flags = flags;
                    }
                    7 => {
                        // CMP
                        let (_, flags) = if prefixes.rex_w {
                            sub64(val, imm)
                        } else {
                            let (r, f) = sub32(val as u32, imm as u32);
                            (r as u64, f)
                        };
                        self.cpu.flags = flags;
                    }
                    _ => return Err(ExitReason::IllegalInstruction(byte)),
                }
                let _ = reg;
                Ok(None)
            }

            // MOV r/m, reg (0x89) or MOV reg, r/m (0x8B)
            0x89 | 0x8B => {
                let modrm = self.fetch_u8()?;
                self.execute_mov_rm(modrm, &prefixes, byte == 0x8B)
            }

            // JMP rel8 (0xEB)
            0xEB => {
                let offset = self.fetch_i8()? as i64;
                self.cpu.rip = (self.cpu.rip as i64 + offset) as u64;
                Ok(None)
            }

            // JMP rel32 (0xE9)
            0xE9 => {
                let offset = self.fetch_i32()? as i64;
                self.cpu.rip = (self.cpu.rip as i64 + offset) as u64;
                Ok(None)
            }

            // JZ/JE rel8 (0x74)
            0x74 => {
                let offset = self.fetch_i8()? as i64;
                if self.cpu.flags.zf {
                    self.cpu.rip = (self.cpu.rip as i64 + offset) as u64;
                }
                Ok(None)
            }

            // JNZ/JNE rel8 (0x75)
            0x75 => {
                let offset = self.fetch_i8()? as i64;
                if !self.cpu.flags.zf {
                    self.cpu.rip = (self.cpu.rip as i64 + offset) as u64;
                }
                Ok(None)
            }

            // CALL rel32 (0xE8)
            0xE8 => {
                let offset = self.fetch_i32()? as i64;
                let target = (self.cpu.rip as i64 + offset) as u64;
                let ret_addr = self.cpu.rip;
                self.push_u64(ret_addr)?;
                self.cpu.rip = target;
                Ok(None)
            }

            // RET (0xC3)
            0xC3 => {
                let ret_addr = self.pop_u64()?;
                self.cpu.rip = ret_addr;
                Ok(None)
            }

            // PUSH reg (0x50..=0x57)
            0x50..=0x57 => {
                let reg_idx = (byte - 0x50) + if prefixes.rex_b { 8 } else { 0 };
                let reg = Register::from_index(reg_idx);
                let val = self.cpu.get(reg);
                self.push_u64(val)?;
                Ok(None)
            }

            // POP reg (0x58..=0x5F)
            0x58..=0x5F => {
                let reg_idx = (byte - 0x58) + if prefixes.rex_b { 8 } else { 0 };
                let reg = Register::from_index(reg_idx);
                let val = self.pop_u64()?;
                self.cpu.set(reg, val);
                Ok(None)
            }

            _ => Err(ExitReason::IllegalInstruction(byte)),
        }
    }

    fn push_u64(&mut self, val: u64) -> Result<(), ExitReason> {
        let sp = self.cpu.get(Register::Rsp);
        let new_sp = sp.checked_sub(8).ok_or(ExitReason::StackUnderflow)?;
        self.memory
            .write_u64(new_sp, val)
            .map_err(ExitReason::MemoryFault)?;
        self.cpu.set(Register::Rsp, new_sp);
        Ok(())
    }

    fn pop_u64(&mut self) -> Result<u64, ExitReason> {
        let sp = self.cpu.get(Register::Rsp);
        let val = self.memory.read_u64(sp).map_err(ExitReason::MemoryFault)?;
        self.cpu.set(Register::Rsp, sp + 8);
        Ok(val)
    }

    fn decode_modrm_reg_reg(
        &self,
        modrm: u8,
        prefixes: &Prefixes,
    ) -> Result<(Register, Register), ExitReason> {
        let mode = (modrm >> 6) & 3;
        if mode != 3 {
            return Err(ExitReason::IllegalInstruction(modrm));
        }
        let reg_idx = ((modrm >> 3) & 7) + if prefixes.rex_r { 8 } else { 0 };
        let rm_idx = (modrm & 7) + if prefixes.rex_b { 8 } else { 0 };
        Ok((Register::from_index(reg_idx), Register::from_index(rm_idx)))
    }

    fn execute_mov_rm(
        &mut self,
        modrm: u8,
        prefixes: &Prefixes,
        is_load: bool,
    ) -> Result<Option<ExitReason>, ExitReason> {
        let mode = (modrm >> 6) & 3;
        let reg_idx = ((modrm >> 3) & 7) + if prefixes.rex_r { 8 } else { 0 };
        let rm_idx = (modrm & 7) + if prefixes.rex_b { 8 } else { 0 };
        let reg = Register::from_index(reg_idx);

        if mode == 3 {
            // Register-to-register move
            let rm = Register::from_index(rm_idx);
            if is_load {
                let val = self.cpu.get(rm);
                self.cpu.set(
                    reg,
                    if prefixes.rex_w {
                        val
                    } else {
                        val as u32 as u64
                    },
                );
            } else {
                let val = self.cpu.get(reg);
                self.cpu.set(
                    rm,
                    if prefixes.rex_w {
                        val
                    } else {
                        val as u32 as u64
                    },
                );
            }
            return Ok(None);
        }

        // Memory operand [rm + disp] or SIB
        let (base_val, disp): (u64, i64) = if (rm_idx & 7) == 4 {
            // SIB byte follows ModR/M
            let sib = self.fetch_u8()?;
            let scale_shift = (sib >> 6) & 3;
            // REX.X extends the SIB index field; REX.R extends the ModRM reg
            // field and must not affect the index. An index field of 100b
            // selects r12 when REX.X is set, and means "no index" otherwise.
            let index_field = (sib >> 3) & 7;
            let index_raw = index_field + if prefixes.rex_x { 8 } else { 0 };
            let base_raw = (sib & 7) + if prefixes.rex_b { 8 } else { 0 };

            let base_v = if mode == 0 && (sib & 7) == 5 {
                // disp32 with no base register
                0u64
            } else {
                self.cpu.get(Register::from_index(base_raw))
            };

            let index_v = if index_field == 4 && !prefixes.rex_x {
                // index == 4 means no index register (ESP/RSP)
                0u64
            } else {
                self.cpu.get(Register::from_index(index_raw)) << scale_shift
            };

            let d: i64 = match mode {
                0 => {
                    if (sib & 7) == 5 {
                        self.fetch_i32()? as i64
                    } else {
                        0
                    }
                }
                1 => self.fetch_i8()? as i64,
                2 => self.fetch_i32()? as i64,
                _ => unreachable!(),
            };

            (base_v.wrapping_add(index_v), d)
        } else {
            let base_reg = Register::from_index(rm_idx);
            let base_val = self.cpu.get(base_reg);

            let disp: i64 = match mode {
                0 => {
                    if (rm_idx & 7) == 5 {
                        // RIP-relative displacement disp32
                        self.fetch_i32()? as i64
                    } else {
                        0
                    }
                }
                1 => self.fetch_i8()? as i64,
                2 => self.fetch_i32()? as i64,
                _ => unreachable!(),
            };
            (base_val, disp)
        };

        let mut effective_addr = if mode == 0 && (rm_idx & 7) == 5 {
            (self.cpu.rip as i64 + disp) as u64
        } else {
            (base_val as i64 + disp) as u64
        };

        if prefixes.seg_gs {
            effective_addr = effective_addr.wrapping_add(self.cpu.gs_base);
        } else if prefixes.seg_fs {
            effective_addr = effective_addr.wrapping_add(self.cpu.fs_base);
        }

        if is_load {
            // Load from memory to reg
            if prefixes.rex_w {
                let val = self
                    .memory
                    .read_u64(effective_addr)
                    .map_err(ExitReason::MemoryFault)?;
                self.cpu.set(reg, val);
            } else {
                let val = self
                    .memory
                    .read_u32(effective_addr)
                    .map_err(ExitReason::MemoryFault)?;
                self.cpu.set(reg, val as u64);
            }
        } else {
            // Store from reg to memory
            let val = self.cpu.get(reg);
            if prefixes.rex_w {
                self.memory
                    .write_u64(effective_addr, val)
                    .map_err(ExitReason::MemoryFault)?;
            } else {
                self.memory
                    .write_u32(effective_addr, val as u32)
                    .map_err(ExitReason::MemoryFault)?;
            }
        }

        Ok(None)
    }

    fn update_flags_logic(&mut self, val: u64, is_64: bool) {
        self.cpu.flags.cf = false;
        self.cpu.flags.of = false;
        if is_64 {
            self.cpu.flags.zf = val == 0;
            self.cpu.flags.sf = (val as i64) < 0;
        } else {
            let val32 = val as u32;
            self.cpu.flags.zf = val32 == 0;
            self.cpu.flags.sf = (val32 as i32) < 0;
        }
    }
}

fn add64(a: u64, b: u64) -> (u64, RFlags) {
    let (res, cf) = a.overflowing_add(b);
    let of = ((a ^ res) & (b ^ res) & 0x8000_0000_0000_0000) != 0;
    let sf = (res as i64) < 0;
    let zf = res == 0;
    (res, RFlags { cf, zf, sf, of })
}

fn add32(a: u32, b: u32) -> (u32, RFlags) {
    let (res, cf) = a.overflowing_add(b);
    let of = ((a ^ res) & (b ^ res) & 0x8000_0000) != 0;
    let sf = (res as i32) < 0;
    let zf = res == 0;
    (res, RFlags { cf, zf, sf, of })
}

fn sub64(a: u64, b: u64) -> (u64, RFlags) {
    let (res, cf) = a.overflowing_sub(b);
    let of = ((a ^ b) & (a ^ res) & 0x8000_0000_0000_0000) != 0;
    let sf = (res as i64) < 0;
    let zf = res == 0;
    (res, RFlags { cf, zf, sf, of })
}

fn sub32(a: u32, b: u32) -> (u32, RFlags) {
    let (res, cf) = a.overflowing_sub(b);
    let of = ((a ^ b) & (a ^ res) & 0x8000_0000) != 0;
    let sf = (res as i32) < 0;
    let zf = res == 0;
    (res, RFlags { cf, zf, sf, of })
}
