// SPDX-License-Identifier: GPL-3.0-or-later
//! Extended x86-64 interpreter: CALL, RET, PUSH, POP, MOV r/m, RSP tracking.
//!
//! # Implemented instructions
//!
//! | Encoding | Mnemonic | Notes |
//! |---|---|---|
//! | `B8+rd id` | `MOV r32, imm32` | Zero-extends to 64 bits; r ∈ RAX–R15 |
//! | `48 B8+rd iq` | `MOV r64, imm64` | Full 64-bit immediate |
//! | `89 /r` | `MOV r/m32, r32` | 32-bit register-to-register |
//! | `48 89 /r` | `MOV r/m64, r64` | 64-bit register-to-register |
//! | `8B /r` | `MOV r32, r/m32` | 32-bit load from memory or register |
//! | `48 8B /r` | `MOV r64, r/m64` | 64-bit memory load |
//! | `48 89 04 24` | `MOV [RSP], r64` | Store into stack slot |
//! | `48 8B 04 24` | `MOV r64, [RSP]` | Load from stack slot |
//! | `50+rd` | `PUSH r64` | Push 64-bit register, RSP -= 8 |
//! | `58+rd` | `POP r64` | Pop 64-bit register, RSP += 8 |
//! | `E8 rel32` | `CALL rel32` | Push return address, jump |
//! | `C3` | `RET` | Pop return address, check code range |
//! | `48 83 C4 imm8` | `ADD RSP, imm8` | Stack pointer adjustment |
//! | `48 83 EC imm8` | `SUB RSP, imm8` | Stack pointer adjustment |
//! | `83 C0 01` | `ADD EAX, 1` | (kept from previous interpreter) |
//! | `83 E8 01` | `SUB EAX, 1` | (kept from previous interpreter) |
//! | `83 F8 imm8` | `CMP EAX, imm8` | (kept from previous interpreter) |
//! | `74 rel8` | `JE rel8` | (kept from previous interpreter) |
//! | `75 rel8` | `JNE rel8` | (kept from previous interpreter) |
//! | `48 89 07` | `MOV [RDI], RAX` | (legacy store; kept for X2–X5 compat) |
//! | `48 89 47 imm8` | `MOV [RDI+imm8], RAX` | Signed-displacement store |
//! | `0F 0B` | `UD2` | Guest service trap |
//! | `EB rel8` | `JMP rel8` | Signed displacement relative to the next instruction |
//!
//! # What is not implemented
//!
//! - Segment registers (FS/GS base, TLS).
//! - SIMD/SSE/AVX (XMM/YMM/ZMM).
//! - FPU (x87) and MMX.
//! - REP prefixes.
//! - LOCK prefixes.
//! - Privileged instructions (SYSCALL, INT, HLT, etc.).
//! - Conditional MOV (CMOVcc).
//! - Most addressing modes beyond register indirect and register+imm8.
//! - Instructions not listed in the table above.
//!
//! # Synthetic call frames
//!
//! CALL pushes the next guest address. RET checks its slot and value against
//! the pending frame and resumes only at an executable address. The entry
//! frame uses an explicit exit on RET. Taken branches may reach any mapped
//! executable byte, where the interpreter decodes a fresh instruction.
//!
//! # Faults
//!
//! | `FaultKind` | Trigger |
//! |---|---|
//! | `CodeRange` | RIP outside the executable code region, or fetch failed |
//! | `Unsupported` | Opcode not in the implemented set |
//! | `Truncated` | Instruction recognised but file truncated |
//! | `BranchTarget` | Branch or CALL target is outside the code region |
//! | `Budget` | Execution step count exhausted |
//! | `MemFault` | Guest memory access denied (unmapped, permissions, etc.) |
//! | `StackFault` | Stack overflow or underflow during PUSH/POP/CALL/RET |
//! | `ReturnFault` | RET address is not in an executable code region |
//! | `UnknownService` | UD2 trap with unregistered service ID |
//!
//! # Limits
//!
//! `MAX_STEPS` caps the total number of retired instructions per execution.
//! This is intentionally conservative to prevent infinite loops from hanging
//! the host.  Set a larger budget (up to `MAX_STEPS`) for programs that need
//! more steps.

use crate::mem::{GuestMemory, MemFault};

pub(crate) const MAX_STEPS: usize = 1024;

// ── register indices (x86-64) ────────────────────────────────────────────────
const RAX: usize = 0;
#[cfg(test)]
const RCX: usize = 1;
const RSP: usize = 4;
const RDI: usize = 7;
// R8–R15 are indices 8–15.

// ── RFLAGS bit positions ─────────────────────────────────────────────────────
const CF: u64 = 1;
const PF: u64 = 1 << 2;
const AF: u64 = 1 << 4;
const ZF: u64 = 1 << 6;
const SF: u64 = 1 << 7;
const OF: u64 = 1 << 11;
const ARITHMETIC: u64 = CF | PF | AF | ZF | SF | OF;

// ── CPU state ────────────────────────────────────────────────────────────────

/// Visible x86-64 register state.
///
/// GPR layout: `[RAX, RCX, RDX, RBX, RSP, RBP, RSI, RDI, R8..R15]`
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct State {
    pub(crate) gpr: [u64; 16],
    pub(crate) rip: u64,
    pub(crate) rflags: u64,
}

impl State {
    /// Construct an initial state: all GPRs zero, RFLAGS = 2 (reserved bit).
    pub(crate) fn new(rip: u64) -> Self {
        Self {
            gpr: [0; 16],
            rip,
            rflags: 2,
        }
    }

    #[inline]
    pub(crate) fn eax(&self) -> u32 {
        self.gpr[RAX] as u32
    }
}

// ── fault types ──────────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FaultKind {
    /// RIP is not in an executable region.
    CodeRange,
    /// Opcode is not in the implemented set.
    Unsupported,
    /// Instruction encoding is truncated.
    Truncated,
    /// Branch or CALL target is outside the executable region.
    BranchTarget,
    /// Execution budget exhausted.
    Budget,
    /// Guest memory access fault.
    MemFault(MemFault),
    /// Stack push or pop hit a guard.
    StackFault,
    /// RET address did not resolve to executable memory.
    ReturnFault,
    /// UD2 trap with unrecognised or unregistered service ID.
    UnknownService,
    UnboundImport,
    InvalidServiceArgument,
    #[cfg(feature = "metal")]
    ComputeFailed,
    #[cfg(feature = "metal")]
    StaleResource,
    #[cfg(feature = "metal")]
    InvalidResource,
    #[cfg(feature = "metal")]
    ResourceLimit,
    #[cfg(feature = "metal")]
    InvalidResourceView,
    #[cfg(feature = "metal")]
    SubmissionPending,
    #[cfg(feature = "metal")]
    NoSubmission,
    #[cfg(feature = "metal")]
    DrawFailed,
    #[cfg(feature = "metal")]
    TextureFailed,
}

impl From<MemFault> for FaultKind {
    fn from(m: MemFault) -> Self {
        match m {
            MemFault::StackOverflow => FaultKind::StackFault,
            other => FaultKind::MemFault(other),
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Fault {
    pub(crate) kind: FaultKind,
    pub(crate) rip: u64,
}

pub(crate) type Service<'a> = dyn FnMut(&State) -> Result<u64, FaultKind> + 'a;
pub(crate) type ImportCall<'a> =
    dyn FnMut(u64, &State, &mut GuestMemory) -> Result<u64, FaultKind> + 'a;

#[derive(Default)]
pub(crate) struct Services<'a, 'callback> {
    pub(crate) trap: Option<&'a mut Service<'callback>>,
    pub(crate) import: Option<&'a mut ImportCall<'callback>>,
}

// ── flag helpers ─────────────────────────────────────────────────────────────

fn result_flags(value: u32) -> u64 {
    let parity = value.to_le_bytes()[0].count_ones().is_multiple_of(2);
    (u64::from(parity) * PF) | (u64::from(value == 0) * ZF) | (u64::from(value >> 31) * SF)
}

fn sub_flags(minuend: u32, subtrahend: u32) -> u64 {
    let result = minuend.wrapping_sub(subtrahend);
    let carry = minuend < subtrahend;
    let af = (minuend & 0xf) < (subtrahend & 0xf);
    let overflow = ((minuend ^ subtrahend) & (minuend ^ result)) >> 31 != 0;
    result_flags(result)
        | (u64::from(carry) * CF)
        | (u64::from(af) * AF)
        | (u64::from(overflow) * OF)
}

// ── ModRM helpers ─────────────────────────────────────────────────────────────

/// Decode a ModRM byte.  Returns `(mod, reg, rm)`.
#[inline]
fn modrm(byte: u8) -> (u8, usize, usize) {
    let modv = byte >> 6;
    let reg = ((byte >> 3) & 7) as usize;
    let rm = (byte & 7) as usize;
    (modv, reg, rm)
}

/// Extend a 3-bit register index with a REX.R or REX.B bit.
#[inline]
fn extend_reg(base: usize, rex_bit: bool) -> usize {
    base | (usize::from(rex_bit) << 3)
}

// ── instruction decode + execute ─────────────────────────────────────────────

/// Execute a supported instruction other than CALL and RET. A zero advance
/// indicates the instruction updated RIP directly.
fn decode_and_execute(
    bytes: &[u8],
    rip: u64,
    state: &mut State,
    mem: &mut GuestMemory,
    service: &mut Option<&mut Service<'_>>,
) -> Result<usize, FaultKind> {
    // We use a macro to read bytes at an offset and return Truncated if OOB.
    macro_rules! byte {
        ($i:expr) => {
            *bytes.get($i).ok_or(FaultKind::Truncated)?
        };
    }
    macro_rules! imm32_at {
        ($i:expr) => {{
            let s = bytes.get($i..$i + 4).ok_or(FaultKind::Truncated)?;
            u32::from_le_bytes(s.try_into().unwrap())
        }};
    }
    macro_rules! imm64_at {
        ($i:expr) => {{
            let s = bytes.get($i..$i + 8).ok_or(FaultKind::Truncated)?;
            u64::from_le_bytes(s.try_into().unwrap())
        }};
    }

    let b0 = byte!(0);

    // ── REX prefix detection ────────────────────────────────────────────────
    let (rex, body_start) = if b0 & 0xF0 == 0x40 {
        (b0, 1usize) // REX prefix
    } else {
        (0u8, 0usize)
    };
    let rex_w = rex & 0x08 != 0; // 64-bit operand
    let rex_r = rex & 0x04 != 0; // extend ModRM.reg
    let rex_b = rex & 0x01 != 0; // extend ModRM.rm / opcode register

    let b1 = if body_start == 0 {
        b0
    } else {
        byte!(body_start)
    };

    // ── Short-form register ops (no ModRM) ───────────────────────────────────

    // PUSH r64 (50+rd) — no REX needed in 64-bit mode.
    if b1 & 0xF8 == 0x50 {
        let reg = extend_reg((b1 & 7) as usize, rex_b);
        let val = state.gpr[reg];
        state.gpr[RSP] = mem.push_u64(state.gpr[RSP], val).map_err(FaultKind::from)?;
        return Ok(body_start + 1);
    }

    // POP r64 (58+rd) — no REX needed in 64-bit mode.
    if b1 & 0xF8 == 0x58 {
        let reg = extend_reg((b1 & 7) as usize, rex_b);
        let (val, new_rsp) = mem.pop_u64(state.gpr[RSP]).map_err(FaultKind::from)?;
        state.gpr[RSP] = new_rsp;
        state.gpr[reg] = val;
        return Ok(body_start + 1);
    }

    // MOV r32/64, imm (B8+rd id/iq).
    if b1 & 0xF8 == 0xB8 {
        let reg = extend_reg((b1 & 7) as usize, rex_b);
        if rex_w {
            // MOV r64, imm64 (REX.W B8+rd iq)
            let imm = imm64_at!(body_start + 1);
            state.gpr[reg] = imm;
            return Ok(body_start + 1 + 8);
        } else {
            // MOV r32, imm32 (B8+rd id) — zero-extends.
            let imm = imm32_at!(body_start + 1);
            state.gpr[reg] = u64::from(imm);
            return Ok(body_start + 1 + 4);
        }
    }

    // ── Remaining opcodes after optional REX ─────────────────────────────────

    match b1 {
        // ── 83 group ────────────────────────────────────────────────────────
        0x83 => {
            let b2 = byte!(body_start + 1);
            let imm = byte!(body_start + 2) as i8;

            if rex == 0x48 && matches!(b2, 0xC4 | 0xEC) {
                let lhs = state.gpr[RSP];
                let rhs = i64::from(imm) as u64;
                let (result, carry, overflow, auxiliary) = if b2 == 0xC4 {
                    let (result, carry) = lhs.overflowing_add(rhs);
                    (
                        result,
                        carry,
                        (!(lhs ^ rhs) & (lhs ^ result)) >> 63,
                        (lhs & 15) + (rhs & 15) > 15,
                    )
                } else {
                    let (result, carry) = lhs.overflowing_sub(rhs);
                    (
                        result,
                        carry,
                        ((lhs ^ rhs) & (lhs ^ result)) >> 63,
                        (lhs & 15) < (rhs & 15),
                    )
                };
                let parity = result.to_le_bytes()[0].count_ones().is_multiple_of(2);
                let flags = (u64::from(carry) * CF)
                    | (u64::from(parity) * PF)
                    | (u64::from(auxiliary) * AF)
                    | (u64::from(result == 0) * ZF)
                    | ((result >> 63) * SF)
                    | (overflow * OF);
                state.gpr[RSP] = result;
                state.rflags = (state.rflags & !ARITHMETIC) | flags;
                return Ok(body_start + 3);
            }

            // 32-bit EAX ops (kept from previous interpreter).
            match b2 {
                0xC0 if imm == 1 && body_start == 0 => {
                    // ADD EAX, 1
                    let eax = state.eax();
                    let (result, carry) = eax.overflowing_add(1);
                    let flags = result_flags(result)
                        | (u64::from(carry) * CF)
                        | (u64::from(eax & 0xf == 0xf) * AF)
                        | (u64::from(eax == 0x7fff_ffff) * OF);
                    state.gpr[RAX] = u64::from(result);
                    state.rflags = (state.rflags & !ARITHMETIC) | flags;
                    Ok(3)
                }
                0xE8 if imm == 1 && body_start == 0 => {
                    // SUB EAX, 1
                    let eax = state.eax();
                    let flags = sub_flags(eax, 1);
                    state.gpr[RAX] = u64::from(eax.wrapping_sub(1));
                    state.rflags = (state.rflags & !ARITHMETIC) | flags;
                    Ok(3)
                }
                0xF8..=0xFF if body_start == 0 => {
                    let subtrahend = i32::from(imm) as u32;
                    let flags = sub_flags(state.gpr[usize::from(b2 & 7)] as u32, subtrahend);
                    state.rflags = (state.rflags & !ARITHMETIC) | flags;
                    Ok(3)
                }
                _ => Err(FaultKind::Unsupported),
            }
        }

        // CMP r32, imm32.
        0x81 if !rex_w => {
            let (mode, extension, register) = modrm(byte!(body_start + 1));
            if mode != 3 || extension != 7 {
                return Err(FaultKind::Unsupported);
            }
            let value = state.gpr[extend_reg(register, rex_b)] as u32;
            let flags = sub_flags(value, imm32_at!(body_start + 2));
            state.rflags = (state.rflags & !ARITHMETIC) | flags;
            Ok(body_start + 6)
        }

        // ── MOV r/m, r  (89 /r) ─────────────────────────────────────────────
        0x89 => {
            let b2 = byte!(body_start + 1);

            // Special case: 48 89 07 → MOV [RDI], RAX (legacy X2–X5 compat).
            if rex == 0x48 && b2 == 0x07 {
                // ModRM: mod=00 reg=RAX rm=RDI → store RAX into [RDI].
                let addr = state.gpr[RDI];
                mem.store_u64(addr, state.gpr[RAX])
                    .map_err(FaultKind::from)?;
                return Ok(body_start + 2);
            }

            // 48 89 47 imm8 → MOV [RDI + imm8], RAX.
            if rex == 0x48 && b2 == 0x47 {
                let disp = byte!(body_start + 2) as i8;
                let addr = state.gpr[RDI].wrapping_add(disp as i64 as u64);
                mem.store_u64(addr, state.gpr[RAX])
                    .map_err(FaultKind::from)?;
                return Ok(body_start + 3);
            }

            // 48 89 04 24 → MOV [RSP], r64.
            if rex_w && !rex_b && rex & 2 == 0 && b2 == 0x04 {
                let b3 = byte!(body_start + 2);
                if b3 == 0x24 {
                    let (_, reg, _) = modrm(b2);
                    let reg = extend_reg(reg, rex_r);
                    let addr = state.gpr[RSP];
                    mem.store_u64(addr, state.gpr[reg])
                        .map_err(FaultKind::from)?;
                    return Ok(body_start + 3);
                }
            }

            let (mode, reg, base) = modrm(b2);
            if mode == 1 && base == 4 && !rex_b && rex & 2 == 0 && byte!(body_start + 2) == 0x24 {
                let address =
                    state.gpr[RSP].wrapping_add_signed(i64::from(byte!(body_start + 3) as i8));
                let value = state.gpr[extend_reg(reg, rex_r)];
                if rex_w {
                    mem.store_u64(address, value)?;
                } else {
                    mem.store_u32(address, value as u32)?;
                }
                return Ok(body_start + 4);
            }

            // General register-to-register MOV.
            let (modv, reg, rm) = modrm(b2);
            let reg = extend_reg(reg, rex_r);
            let rm = extend_reg(rm, rex_b);
            if modv == 3 {
                // Both operands are registers.
                if rex_w {
                    state.gpr[rm] = state.gpr[reg];
                } else {
                    state.gpr[rm] = u64::from(state.gpr[reg] as u32);
                }
                return Ok(body_start + 2);
            }
            Err(FaultKind::Unsupported)
        }

        // ── MOV r, r/m  (8B /r) ─────────────────────────────────────────────
        0x8B => {
            let b2 = byte!(body_start + 1);

            // 48 8B 04 24 → MOV r64, [RSP].
            if rex_w && !rex_b && rex & 2 == 0 && b2 == 0x04 {
                let b3 = byte!(body_start + 2);
                if b3 == 0x24 {
                    let (_, reg, _) = modrm(b2);
                    let reg = extend_reg(reg, rex_r);
                    let addr = state.gpr[RSP];
                    state.gpr[reg] = mem.load_u64(addr).map_err(FaultKind::from)?;
                    return Ok(body_start + 3);
                }
            }

            let (modv, reg, rm) = modrm(b2);
            let reg = extend_reg(reg, rex_r);
            let rm = extend_reg(rm, rex_b);

            if modv == 3 {
                // Register-to-register.
                if rex_w {
                    state.gpr[reg] = state.gpr[rm];
                } else {
                    state.gpr[reg] = u64::from(state.gpr[rm] as u32);
                }
                return Ok(body_start + 2);
            }

            if modv == 1 && rm == RSP && rex & 2 == 0 && byte!(body_start + 2) == 0x24 {
                let address =
                    state.gpr[RSP].wrapping_add_signed(i64::from(byte!(body_start + 3) as i8));
                state.gpr[reg] = if rex_w {
                    mem.load_u64(address)?
                } else {
                    u64::from(mem.load_u32(address)?)
                };
                return Ok(body_start + 4);
            }

            if modv == 2 && rm & 7 != 4 {
                // [reg + disp32] — handled for indirect loads.
                let disp = imm32_at!(body_start + 2) as i32;
                let base = rm;
                let addr = state.gpr[base].wrapping_add(disp as i64 as u64);
                if rex_w {
                    state.gpr[reg] = mem.load_u64(addr).map_err(FaultKind::from)?;
                } else {
                    state.gpr[reg] = u64::from(mem.load_u32(addr).map_err(FaultKind::from)?);
                }
                return Ok(body_start + 6);
            }

            if modv == 1 && rm & 7 != 4 {
                // [reg + disp8].
                let disp = byte!(body_start + 2) as i8;
                let base = rm;
                let addr = state.gpr[base].wrapping_add(disp as i64 as u64);
                if rex_w {
                    state.gpr[reg] = mem.load_u64(addr).map_err(FaultKind::from)?;
                } else {
                    state.gpr[reg] = u64::from(mem.load_u32(addr).map_err(FaultKind::from)?);
                }
                return Ok(body_start + 3);
            }

            Err(FaultKind::Unsupported)
        }

        0xC7 if body_start == 0 && byte!(1) == 0x44 && byte!(2) == 0x24 => {
            let address = state.gpr[RSP].wrapping_add_signed(i64::from(byte!(3) as i8));
            mem.store_u32(address, imm32_at!(4))?;
            Ok(8)
        }
        0x8D if rex_w && !rex_b && rex & 2 == 0 => {
            let (mode, reg, base) = modrm(byte!(body_start + 1));
            if mode != 1 || base != 4 || byte!(body_start + 2) != 0x24 {
                return Err(FaultKind::Unsupported);
            }
            state.gpr[extend_reg(reg, rex_r)] =
                state.gpr[RSP].wrapping_add_signed(i64::from(byte!(body_start + 3) as i8));
            Ok(body_start + 4)
        }

        0x74 | 0x75 | 0xEB if body_start == 0 => {
            let disp = i64::from(byte!(1) as i8);
            let zero = state.rflags & ZF != 0;
            if b1 == 0xEB || (b1 == 0x74) == zero {
                let (_, target) = relative_target(mem, rip, 2, disp)?;
                state.rip = target;
                Ok(0)
            } else {
                Ok(2)
            }
        }

        // ── 0F group ────────────────────────────────────────────────────────
        0x0F => {
            let b2 = byte!(body_start + 1);
            match b2 {
                // UD2 — guest service trap.
                0x0B => {
                    let handler = service.as_mut().ok_or(FaultKind::UnknownService)?;
                    state.gpr[RAX] = handler(state)?;
                    Ok(body_start + 2)
                }
                0x84 | 0x85 if body_start == 0 => {
                    let displacement = i64::from(imm32_at!(2) as i32);
                    let zero = state.rflags & ZF != 0;
                    if (b2 == 0x84) == zero {
                        let (_, target) = relative_target(mem, rip, 6, displacement)?;
                        state.rip = target;
                        Ok(0)
                    } else {
                        Ok(6)
                    }
                }
                _ => Err(FaultKind::Unsupported),
            }
        }

        _ => Err(FaultKind::Unsupported),
    }
}

// ── top-level interpreter ────────────────────────────────────────────────────

fn relative_target(
    mem: &GuestMemory,
    rip: u64,
    length: u64,
    displacement: i64,
) -> Result<(u64, u64), FaultKind> {
    let next = rip.checked_add(length).ok_or(FaultKind::BranchTarget)?;
    let target = next
        .checked_add_signed(displacement)
        .ok_or(FaultKind::BranchTarget)?;
    mem.fetch(target, 1).map_err(|_| FaultKind::BranchTarget)?;
    Ok((next, target))
}

/// Execute from the supplied RIP and stack top. CALL frames track the expected
/// stack slot and continuation independently of guest memory. Only RET from
/// the root frame may exit and it must consume the original root slot.
pub(crate) fn run(
    mem: &mut GuestMemory,
    state: &mut State,
    budget: usize,
    service: Option<&mut Service<'_>>,
    retired: impl FnMut(&State),
) -> Result<(), Fault> {
    run_with_services(
        mem,
        state,
        budget,
        Services {
            trap: service,
            import: None,
        },
        retired,
    )
}

pub(crate) fn run_with_services(
    mem: &mut GuestMemory,
    state: &mut State,
    budget: usize,
    mut services: Services<'_, '_>,
    mut retired: impl FnMut(&State),
) -> Result<(), Fault> {
    let root_slot = mem.push_u64(state.gpr[RSP], 0).map_err(|kind| Fault {
        kind: kind.into(),
        rip: state.rip,
    })?;
    state.gpr[RSP] = root_slot;
    let mut calls = Vec::new();
    for _ in 0..budget.min(MAX_STEPS) {
        let rip = state.rip;
        let bytes = mem.fetch(rip, 15).map_err(|_| Fault {
            kind: FaultKind::CodeRange,
            rip,
        })?;
        let mut instruction = [0; 15];
        instruction[..bytes.len()].copy_from_slice(bytes);
        let instruction = &instruction[..bytes.len()];

        let mut step = || -> Result<bool, FaultKind> {
            match instruction.first() {
                Some(0xE8) => {
                    let rel = instruction.get(1..5).ok_or(FaultKind::Truncated)?;
                    let displacement = i32::from_le_bytes([rel[0], rel[1], rel[2], rel[3]]);
                    let (next, target) = relative_target(mem, rip, 5, i64::from(displacement))?;
                    let slot = mem.push_u64(state.gpr[RSP], next)?;
                    calls.push((slot, next));
                    state.gpr[RSP] = slot;
                    state.rip = target;
                }
                Some(0xFF) if instruction.get(1) == Some(&0x15) => {
                    let call = services.import.as_mut().ok_or(FaultKind::Unsupported)?;
                    let rel = instruction.get(2..6).ok_or(FaultKind::Truncated)?;
                    let displacement = i32::from_le_bytes([rel[0], rel[1], rel[2], rel[3]]);
                    let next = rip.checked_add(6).ok_or(FaultKind::BranchTarget)?;
                    let slot = next
                        .checked_add_signed(i64::from(displacement))
                        .ok_or(FaultKind::BranchTarget)?;
                    mem.fetch(next, 1).map_err(|_| FaultKind::ReturnFault)?;
                    mem.check_import_frame(state.gpr[RSP])?;
                    state.gpr[RAX] = call(slot, state, mem)?;
                    state.rip = next;
                }
                Some(0xC3) => {
                    let slot = state.gpr[RSP];
                    let (expected_slot, expected_return) =
                        calls.last().copied().unwrap_or((root_slot, 0));
                    if slot != expected_slot {
                        return Err(FaultKind::ReturnFault);
                    }
                    let (target, new_rsp) = mem.pop_u64(slot)?;
                    if target != expected_return {
                        return Err(FaultKind::ReturnFault);
                    }
                    if calls.is_empty() {
                        state.gpr[RSP] = new_rsp;
                        return Ok(true);
                    }
                    mem.fetch(target, 1).map_err(|_| FaultKind::ReturnFault)?;
                    calls.pop();
                    state.gpr[RSP] = new_rsp;
                    state.rip = target;
                }
                _ => {
                    let consumed =
                        decode_and_execute(instruction, rip, state, mem, &mut services.trap)?;
                    if consumed != 0 {
                        state.rip = rip
                            .checked_add(consumed as u64)
                            .ok_or(FaultKind::CodeRange)?;
                    }
                }
            }
            Ok(false)
        };
        let exit = (step)().map_err(|kind| Fault { kind, rip })?;
        retired(state);
        if exit {
            return Ok(());
        }
    }
    Err(Fault {
        kind: FaultKind::Budget,
        rip: state.rip,
    })
}

// ── tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mem::{GuestMemory, Perm};

    const CODE_BASE: u64 = 0x1_4000_1000;
    const DATA_BASE: u64 = 0x1_4000_2000;
    const STACK_BASE: u64 = 0x7FFF_F000;
    const STACK_SIZE: usize = 0x2000;

    /// Build a GuestMemory with code mapped at CODE_BASE, 8-byte data at
    /// DATA_BASE, and a stack.  Returns (mem, initial_rsp).
    fn setup(code: &[u8]) -> (GuestMemory, u64) {
        let mut mem = GuestMemory::new();
        // Map code region.
        assert!(mem.map(CODE_BASE, code.len(), Perm::RX, 0));
        mem.initialize(CODE_BASE, code).unwrap();
        // Map data region.
        assert!(mem.map(DATA_BASE, 0x1000, Perm::RW, 0xA5));
        // Allocate stack.
        let rsp = mem.alloc_stack(STACK_BASE, STACK_SIZE).unwrap();
        (mem, rsp)
    }

    fn run_code(code: &[u8], budget: usize) -> Result<([u64; 16], u64), Fault> {
        let (mut mem, rsp) = setup(code);
        let mut state = State::new(CODE_BASE);
        state.gpr[RSP] = rsp;
        run(&mut mem, &mut state, budget, None, |_| {})?;
        Ok((state.gpr, state.rflags))
    }

    #[test]
    fn mov_eax_imm32_zero_extends() {
        // B8 FF FF FF 7F  MOV EAX, 0x7FFFFFFF
        // C3              RET
        let code = [0xB8u8, 0xFF, 0xFF, 0xFF, 0x7F, 0xC3];
        let (gpr, _) = run_code(&code, 10).unwrap();
        assert_eq!(gpr[RAX], 0x7FFF_FFFF);
    }

    #[test]
    fn mov_r64_imm64() {
        // 48 B8 EF CD AB 89 67 45 23 01  MOV RAX, 0x0123456789ABCDEF
        // C3                              RET
        let mut code = vec![0x48u8, 0xB8];
        code.extend_from_slice(&0x0123_4567_89AB_CDEFu64.to_le_bytes());
        code.push(0xC3);
        let (gpr, _) = run_code(&code, 10).unwrap();
        assert_eq!(gpr[RAX], 0x0123_4567_89AB_CDEF);
    }

    #[test]
    fn push_pop_roundtrip() {
        // B8 42 00 00 00  MOV EAX, 42
        // 50              PUSH RAX
        // B8 00 00 00 00  MOV EAX, 0
        // 58              POP RAX
        // C3              RET
        let code = [
            0xB8, 42, 0, 0, 0,    // MOV EAX, 42
            0x50, // PUSH RAX
            0xB8, 0, 0, 0, 0,    // MOV EAX, 0
            0x58, // POP RAX
            0xC3, // RET
        ];
        let (gpr, _) = run_code(&code, 20).unwrap();
        assert_eq!(gpr[RAX], 42);
    }

    #[test]
    fn call_ret_executes_subfunction() {
        // Simple call: call add_one which does ADD EAX,1 then RET.
        //
        // main:
        //   B8 05 00 00 00    MOV EAX, 5
        //   E8 02 00 00 00    CALL add_one   ; rel32 = 2 (next_rip = CODE_BASE+10, target = CODE_BASE+12)
        //   C3                RET            ; return from the root frame
        //   90 90             (padding; rel displacement skips these)
        // add_one:            ; offset 12 = 0xC from CODE_BASE
        //   83 C0 01          ADD EAX, 1
        //   C3                RET
        //
        // CALL E8: encoding E8 rel32 where rel = target - next_rip.
        // next_rip = CODE_BASE + 10; target = CODE_BASE + 12 → rel = 2.
        let code = [
            0xB8, 5, 0, 0, 0, // 0–4: MOV EAX, 5
            0xE8, 3, 0, 0, 0,    // 5–9: CALL +3 → target=CODE_BASE+13
            0xC3, // 10: RET
            0x90, 0x90, // 11,12: NOP padding
            0x83, 0xC0, 0x01, // 13–15: ADD EAX, 1
            0xC3, // 16: RET
        ];
        let (gpr, _) = run_code(&code, 20).unwrap();
        assert_eq!(gpr[RAX], 6);
    }

    #[test]
    fn nested_calls_return_correctly() {
        // main:   CALL outer → outer CALL inner → inner RET → outer RET → main RET → exit.
        // inner: MOV EAX, 99; RET
        // outer: CALL inner; ADD EAX, 1; RET
        // main:  MOV EAX, 0; CALL outer; RET
        //
        // Layout (all at CODE_BASE):
        // [0]  main_start:
        //   B8 00 00 00 00   ; 0: MOV EAX, 0
        //   E8 ??            ; 5: CALL outer
        //   C3               ; 10: RET → exit
        //   90 90            ; 11,12: pad
        // [13] outer:
        //   E8 ??            ; 13: CALL inner
        //   83 C0 01         ; 18: ADD EAX, 1
        //   C3               ; 21: RET → back to main+10
        //   90               ; 22: pad
        // [23] inner:
        //   B8 63 00 00 00   ; 23: MOV EAX, 99
        //   C3               ; 28: RET → back to outer+5(=18)
        //
        // CALL outer: at offset 5, next_rip = 10, target = 13 → rel = 3.
        // CALL inner: at offset 13, next_rip = 18, target = 23 → rel = 5.
        let code = [
            // main
            0xB8, 0, 0, 0, 0, // 0: MOV EAX, 0
            0xE8, 3, 0, 0, 0,    // 5: CALL outer (rel=3, next=10, target=13)
            0xC3, // 10: RET
            0x90, 0x90, // 11,12: pad
            // outer at 13
            0xE8, 5, 0, 0, 0, // 13: CALL inner (rel=5, next=18, target=23)
            0x83, 0xC0, 0x01, // 18: ADD EAX, 1
            0xC3, // 21: RET
            0x90, // 22: pad
            // inner at 23
            0xB8, 99, 0, 0, 0,    // 23: MOV EAX, 99
            0xC3, // 28: RET
        ];
        let (gpr, _) = run_code(&code, 30).unwrap();
        assert_eq!(gpr[RAX], 100);
    }

    #[test]
    fn cmp_je_jne_branching() {
        // MOV EAX, 5; CMP EAX, 5; JE +1; (NOP skipped); RET;
        // 0: B8 05 00 00 00  MOV EAX,5
        // 5: 83 F8 05        CMP EAX,5    sets ZF=1
        // 8: 74 01           JE  +1       → jumps to 11
        // 10: 90             NOP (skipped)
        // 11: C3             RET
        let code = [
            0xB8, 5, 0, 0, 0, // 0: MOV EAX, 5
            0x83, 0xF8, 5, // 5: CMP EAX, 5
            0x74, 1,    // 8: JE +1 → 11
            0x90, // 10: NOP (should be skipped)
            0xC3, // 11: RET
        ];
        let (gpr, _) = run_code(&code, 20).unwrap();
        assert_eq!(gpr[RAX], 5); // unchanged
    }

    #[test]
    fn budget_exhaustion_returns_fault() {
        // Infinite loop: JNE -2 (jump back to itself when ZF=0, which is always true for
        // mismatched values).
        // 83 F8 01  CMP EAX, 1   (EAX=0 initially, ZF=0)
        // 75 FD     JNE -3       → back to CMP
        let code = [
            0x83, 0xF8, 1, // CMP EAX, 1
            0x75, 0xFBu8, // JNE -5 → back to offset 0
        ];
        let (mut mem, rsp) = setup(&code);
        let mut state = State::new(CODE_BASE);
        state.gpr[RSP] = rsp;
        let result = run(&mut mem, &mut state, 50, None, |_| {});
        assert!(matches!(
            result,
            Err(Fault {
                kind: FaultKind::Budget,
                ..
            })
        ));
    }

    #[test]
    fn root_ret_retires_on_the_last_budget_slot() {
        let (gpr, _) = run_code(&[0xC3], 1).unwrap();
        assert_eq!(gpr[RSP], STACK_BASE + STACK_SIZE as u64);
    }

    #[test]
    fn nested_return_cannot_consume_the_root_slot() {
        // CALL to offset 5, then skip the nested return slot before RET.
        let code = [0xE8, 0, 0, 0, 0, 0x48, 0x83, 0xC4, 8, 0xC3];
        assert_eq!(
            run_code(&code, 8),
            Err(Fault {
                kind: FaultKind::ReturnFault,
                rip: CODE_BASE + 9,
            })
        );
    }

    #[test]
    fn corrupt_root_slot_faults_even_if_new_target_is_executable() {
        let mut code = vec![0x48, 0xB8];
        code.extend(CODE_BASE.to_le_bytes());
        code.extend([0x48, 0x89, 0x04, 0x24, 0xC3]);
        assert_eq!(
            run_code(&code, 8),
            Err(Fault {
                kind: FaultKind::ReturnFault,
                rip: CODE_BASE + 14,
            })
        );
    }

    #[test]
    fn jmp_accepts_a_decodable_target_inside_other_instruction_bytes() {
        // JMP +1 lands in a would-be MOV immediate at offset 3.
        assert!(run_code(&[0xEB, 1, 0xB8, 0xC3], 2).is_ok());
    }

    #[test]
    fn jmp_and_call_reject_truncated_and_unmapped_targets() {
        for (code, kind) in [
            (&[0xEB][..], FaultKind::Truncated),
            (&[0xE8, 0][..], FaultKind::Truncated),
            (&[0xEB, 0][..], FaultKind::BranchTarget),
            (&[0xEB, 0xFD][..], FaultKind::BranchTarget),
            (&[0xE8, 0, 0, 0, 0][..], FaultKind::BranchTarget),
        ] {
            assert_eq!(
                run_code(code, 1),
                Err(Fault {
                    kind,
                    rip: CODE_BASE
                })
            );
        }
    }

    #[test]
    fn invalid_call_leaves_the_return_slot_and_stack_pointer_unchanged() {
        let (mut mem, rsp) = setup(&[0xE8, 0, 0, 0, 0]);
        let mut state = State::new(CODE_BASE);
        state.gpr[RSP] = rsp;
        mem.store_u64(rsp - 16, 42).unwrap();
        assert_eq!(
            run(&mut mem, &mut state, 1, None, |_| {}).unwrap_err().kind,
            FaultKind::BranchTarget
        );
        assert_eq!(state.gpr[RSP], rsp - 8);
        assert_eq!(mem.load_u64(rsp - 16), Ok(42));
    }

    #[test]
    fn backward_jmp_exhausts_the_budget() {
        assert_eq!(
            run_code(&[0xEB, 0xFE], 3),
            Err(Fault {
                kind: FaultKind::Budget,
                rip: CODE_BASE,
            })
        );
    }

    #[test]
    fn unknown_service_faults_at_the_trap() {
        assert_eq!(
            run_code(&[0x0F, 0x0B], 2),
            Err(Fault {
                kind: FaultKind::UnknownService,
                rip: CODE_BASE,
            })
        );
    }

    #[test]
    fn invalid_return_address_faults() {
        // Build a program that manually pushes a bad return address.
        // 48 B8 imm64  MOV RAX, 0xDEAD (not in code region)
        // 50           PUSH RAX
        // C3           RET   → should fault ReturnFault
        let bad_addr: u64 = 0xDEAD_DEAD_DEAD_DEAD;
        let mut code = vec![0x48u8, 0xB8];
        code.extend_from_slice(&bad_addr.to_le_bytes());
        code.extend_from_slice(&[0x50, 0xC3]);
        let (mut mem, rsp) = setup(&code);
        let mut state = State::new(CODE_BASE);
        state.gpr[RSP] = rsp;
        let result = run(&mut mem, &mut state, 20, None, |_| {});
        assert!(matches!(
            result,
            Err(Fault {
                kind: FaultKind::ReturnFault,
                ..
            })
        ));
    }

    #[test]
    fn stack_write_read_via_mov_rsp_indirect() {
        // 48 B8 …     MOV RAX, 0xCAFE
        // 48 89 04 24 MOV [RSP], RAX   (store to stack)
        // 48 B8 …     MOV RAX, 0
        // 48 8B 04 24 MOV RAX, [RSP]   (load from stack)
        // C3          RET
        let val: u64 = 0xCAFE_BABE;
        let mut code = vec![0x48, 0x83, 0xEC, 8, 0x48, 0xB8];
        code.extend_from_slice(&val.to_le_bytes());
        code.extend_from_slice(&[0x48, 0x89, 0x04, 0x24]); // MOV [RSP], RAX
        code.extend_from_slice(&[0x48, 0xB8]);
        code.extend_from_slice(&0u64.to_le_bytes()); // MOV RAX, 0
        code.extend_from_slice(&[0x48, 0x8B, 0x04, 0x24]); // MOV RAX, [RSP]
        code.extend_from_slice(&[0x48, 0x83, 0xC4, 8, 0xC3]); // RET
        let (gpr, _) = run_code(&code, 20).unwrap();
        assert_eq!(gpr[RAX], val);
    }

    #[test]
    fn ud2_service_trap_dispatches() {
        // B8 07 00 00 00  MOV EAX, 7   (service ID)
        // 0F 0B           UD2          (trap)
        // C3              RET
        let code = [0xB8u8, 7, 0, 0, 0, 0x0F, 0x0B, 0xC3];
        let (mut mem, rsp) = setup(&code);
        let mut state = State::new(CODE_BASE);
        state.gpr[RSP] = rsp;
        let mut called = false;
        let mut svc = |s: &State| -> Result<u64, FaultKind> {
            assert_eq!(s.gpr[RAX], 7);
            called = true;
            Ok(42)
        };
        run(&mut mem, &mut state, 10, Some(&mut svc), |_| {}).unwrap();
        assert!(called);
        assert_eq!(state.gpr[RAX], 42);
    }

    #[test]
    fn rsp_adjustment_add_sub() {
        // 48 83 EC 10  SUB RSP, 16
        // 48 83 C4 10  ADD RSP, 16
        // C3           RET
        let code = [
            0x48, 0x83, 0xEC, 16, // SUB RSP, 16
            0x48, 0x83, 0xC4, 16,   // ADD RSP, 16
            0xC3, // RET
        ];
        let (mut mem, rsp) = setup(&code);
        let mut state = State::new(CODE_BASE);
        state.gpr[RSP] = rsp;
        let initial_rsp = state.gpr[RSP];
        run(&mut mem, &mut state, 10, None, |_| {}).unwrap();
        // After SUB+ADD RSP should be back to initial_rsp.
        assert_eq!(state.gpr[RSP], initial_rsp);
    }

    #[test]
    fn unsupported_rex_and_sib_forms_fault_instead_of_aliasing_registers() {
        for code in [
            &[0x49, 0x83, 0xEC, 8][..],       // SUB R12, 8
            &[0x49, 0x89, 0x07][..],          // MOV [R15], RAX
            &[0x4A, 0x8B, 0x04, 0x24][..],    // MOV RAX, [RSP + R12]
            &[0x48, 0x8B, 0x44, 0x2C, 8][..], // MOV RAX, [RSP + RBP + 8] — unsupported indexed SIB
        ] {
            assert_eq!(
                run_code(code, 1),
                Err(Fault {
                    kind: FaultKind::Unsupported,
                    rip: CODE_BASE
                })
            );
        }
    }

    #[test]
    fn rsp_adjustment_updates_flags_for_the_next_branch() {
        let code = [
            0x83, 0xF8, 0, // CMP EAX, 0 sets ZF
            0x48, 0x83, 0xEC, 8, // SUB RSP, 8 clears ZF
            0x74, 4, // JE to RET would use the wrong slot
            0x48, 0x83, 0xC4, 8, // ADD RSP, 8 restores the root slot
            0xC3,
        ];
        let (gpr, flags) = run_code(&code, 5).unwrap();
        assert_eq!(gpr[RSP], STACK_BASE + STACK_SIZE as u64);
        assert_eq!(flags & ZF, 0);
    }

    #[test]
    fn mov_reg_to_reg_32bit() {
        // B8 FF FF FF FF   MOV EAX, 0xFFFFFFFF
        // 89 C1            MOV ECX, EAX   (zero-extends, so RCX = 0x00000000_FFFFFFFF)
        // C3               RET
        let code = [
            0xB8, 0xFF, 0xFF, 0xFF, 0xFF, // MOV EAX, 0xFFFFFFFF
            0x89, 0xC1, // MOV ECX, EAX
            0xC3, // RET
        ];
        let (gpr, _) = run_code(&code, 10).unwrap();
        assert_eq!(gpr[RCX], 0x0000_0000_FFFF_FFFF);
    }

    #[test]
    fn mov_reg_to_reg_64bit() {
        // 48 B8 imm64      MOV RAX, 0xDEADBEEFCAFEBABE
        // 48 89 C1         MOV RCX, RAX
        // C3               RET
        let val: u64 = 0xDEAD_BEEF_CAFE_BABE;
        let mut code = vec![0x48u8, 0xB8];
        code.extend_from_slice(&val.to_le_bytes());
        code.extend_from_slice(&[0x48, 0x89, 0xC1, 0xC3]);
        let (gpr, _) = run_code(&code, 10).unwrap();
        assert_eq!(gpr[RCX], val);
    }
}
