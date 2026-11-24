// SPDX-License-Identifier: GPL-3.0-or-later

//! A second, structurally independent interpreter for the demo instruction
//! subset, used only by the semantic-review tests.
//!
//! This module deliberately imports neither `xenora-decoder`, `xenora-isa` nor
//! `xenora-ref`. It re-decodes the gfx1030 encodings from the AMD RDNA 2 ISA
//! field layout and executes them with an explicit program-counter machine, so
//! a defect in the production decoder or reference cannot cancel out here.
//!
//! Scope matches the demo fixtures: scalar move, the twelve SOPC 32-bit integer
//! compares, branch, SOP2 unsigned
//! and signed add and subtract, add-with-carry, subtract-with-borrow, AND, OR
//! and XOR, the stage C forms ANDN2, ORN2, NAND, NOR, XNOR, LSHL, LSHR and
//! ASHR (32-bit), VOP1 move
//! (including from an SGPR),
//! VOP2 bitwise, shift, arithmetic-shift and float, VOP3 add-no-carry, MUBUF
//! load/store and the two waits plus `s_endpgm`. Anything else is a controlled
//! error. The arithmetic shift is evaluated as a floor division by a power of
//! two rather than with a host signed shift, so the oracle does not share the
//! production operator. SOP2 arithmetic runs through a bit-serial ripple
//! adder (subtraction as addition of the complement), not host integer ops.
//!
//! Wave32 VOPC integer compares (`v_cmp_{lt,eq,le,gt,ne,ge}_{i32,u32}_e32`,
//! contract `docs/shaders/vector-compare.md`) need all lanes of a wave, so execution
//! runs lockstep waves of 32 consecutive dispatch indices with wave-uniform
//! scalar state and per-lane vector state. A lane-local failure stops only that
//! lane and the lowest-index error is reported, so programs without compares
//! fail exactly as independent per-lane runs did. Signed compares flip the sign
//! bit of both operands and then compare unsigned. VCC_LO is uninitialized,
//! known or lane-derived; a lane-derived VCC_LO feeds `s_cbranch_vccz` only
//! after a static check of the skipped region.

const MAX_CODE_BYTES: usize = 1024;
const MAX_INPUT_WORDS: usize = 4096;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Src {
    Reg(u8),
    Imm(u32),
}

/// A VOPC SRC0 operand (contract 2 allowlist).
#[derive(Clone, Copy, Debug)]
enum CmpSrc {
    Sgpr(u8),
    Vcc,
    Imm(u32),
    Vgpr(u8),
}

#[derive(Clone, Copy, Debug)]
enum Insn {
    SMov(u8, Src),
    /// SOP2 opcode 0-5, 14, 16, 18 or an even opcode 20-34.
    SAlu(u32, u8, Src, Src),
    /// SOPC opcode 0-11: `s_cmp_{eq,lg,gt,ge,lt,le}_i32` then `_u32`.
    SCmp(u32, Src, Src),
    BranchScc0(usize),
    BranchMaskZero {
        target: usize,
        register: usize,
    },
    Branch(usize),
    VMov(u8, Src),
    VMovFromSgpr(u8, u8),
    /// `v_mov_b32_e32 vD, vcc_lo`.
    VMovFromVcc(u8),
    /// VOPC compare into VCC_LO: relation 1 lt, 2 eq, 3 le, 4 gt, 5 ne, 6 ge.
    VCmp {
        relation: u8,
        signed: bool,
        src0: CmpSrc,
        src1: u8,
    },
    VAddNc(u8, u8, u8),
    VAnd(u8, u8, u8),
    VOr(u8, u8, u8),
    VShiftLeft(u8, u8, u8),
    VShiftRight(u8, u8, u8),
    VShiftRightArithmetic(u8, u8, u8),
    VXor(u8, u8, u8),
    VAddF(u8, u8, u8),
    VMulF(u8, u8, u8),
    Load(u8),
    Store(u8),
    WaitRead,
    WaitWrite,
    End,
}

/// Execute the synthetic fixture ABI and return one output word per input word.
pub fn execute(code: &[u8], input: &[u32]) -> Result<Vec<u32>, String> {
    if input.is_empty() || input.len() > MAX_INPUT_WORDS {
        return Err("input word count outside limits".to_string());
    }
    if code.len() > MAX_CODE_BYTES {
        return Err("code exceeds limit".to_string());
    }
    let program = decode(code)?;
    let mut output = Vec::with_capacity(input.len());
    for wave in input.chunks(32) {
        output.extend(run_wave(&program, wave)?);
    }
    Ok(output)
}

/// Per-lane vector and memory state of one wave lane.
struct Lane {
    input: u32,
    vgpr: [Option<u32>; 256],
    pending_read: Option<u8>,
    pending_write: Option<u32>,
    written: Option<u32>,
    /// A lane-local failure; the lane stops executing.
    error: Option<String>,
}

/// The lowest-index failed lane's error, in dispatch order.
fn first_error(lanes: &[Lane]) -> Option<String> {
    lanes.iter().find_map(|lane| lane.error.clone())
}

/// Fail every still-running lane with a wave-uniform error and report the
/// lowest-index failure, exactly as independent per-lane runs would.
fn wave_error(lanes: &mut [Lane], error: String) -> String {
    for lane in lanes.iter_mut() {
        lane.error.get_or_insert_with(|| error.clone());
    }
    first_error(lanes).unwrap_or(error)
}

fn run_wave(program: &[(usize, Insn)], wave: &[u32]) -> Result<Vec<u32>, String> {
    let mut lanes: Vec<Lane> = wave
        .iter()
        .map(|&input| Lane {
            input,
            vgpr: [None; 256],
            pending_read: None,
            pending_write: None,
            written: None,
            error: None,
        })
        .collect();
    // Scalar state is wave-uniform and shared by all lanes.
    let mut sgpr = [None::<u32>; 128];
    sgpr[126] = Some(u32::MAX);
    let mut scc: Option<bool> = None;
    // sgpr[106] holds VCC_LO's value; this flag marks it lane-derived.
    let mut vcc_lane_derived = false;
    let mut pc = 0usize;

    while pc < program.len() {
        let (offset, insn) = program[pc];
        if lanes.iter().all(|lane| lane.error.is_some()) {
            break;
        }
        let read_s = |sgpr: &[Option<u32>; 128], src: Src| -> Result<u32, String> {
            match src {
                Src::Imm(value) => Ok(value),
                Src::Reg(reg) => {
                    sgpr[reg as usize].ok_or_else(|| format!("{offset:#x}: uninitialized s{reg}"))
                }
            }
        };
        let execution_mask = match read_s(&sgpr, Src::Reg(126)) {
            Ok(mask) => mask,
            Err(error) => return Err(wave_error(&mut lanes, error)),
        };

        let scalar: Result<Option<usize>, String> = match insn {
            Insn::End => {
                for lane in lanes.iter_mut().filter(|lane| lane.error.is_none()) {
                    if lane.pending_read.is_some() || lane.pending_write.is_some() {
                        lane.error = Some(format!("{offset:#x}: outstanding transfer at end"));
                    } else if lane.written.is_none() {
                        lane.error = Some(format!("{offset:#x}: missing output store"));
                    }
                }
                if let Some(error) = first_error(&lanes) {
                    return Err(error);
                }
                return Ok(lanes.iter().map(|lane| lane.written.unwrap()).collect());
            }
            Insn::VCmp {
                relation,
                signed,
                src0,
                src1,
            } => {
                // A compare couples lanes; a failed lane has no defined
                // contribution, so the wave stops with its error.
                if let Some(error) = first_error(&lanes) {
                    return Err(error);
                }
                compare(
                    &lanes,
                    &sgpr,
                    execution_mask,
                    (relation, signed, src0, src1),
                    offset,
                )
                .map(|mask| {
                    sgpr[106] = Some(mask);
                    // VCC_HI has no known value after a compare (contract 3).
                    sgpr[107] = None;
                    vcc_lane_derived = execution_mask != 0;
                    None
                })
            }
            Insn::SMov(dst, src) => {
                if dst < 8 {
                    Err(format!("{offset:#x}: write to a resource SGPR"))
                } else {
                    read_s(&sgpr, src).map(|value| {
                        sgpr[dst as usize] = Some(value);
                        if dst == 106 {
                            vcc_lane_derived = false;
                        }
                        None
                    })
                }
            }
            Insn::SCmp(opcode, a, b) => read_s(&sgpr, a).and_then(|a| {
                read_s(&sgpr, b).map(|b| {
                    scc = Some(scalar_compare(opcode, a, b));
                    None
                })
            }),
            Insn::SAlu(opcode, dst, a, b) => {
                scalar_alu(&mut sgpr, &mut scc, (opcode, dst, a, b), offset).map(|()| None)
            }
            Insn::BranchScc0(target) => scc
                .ok_or_else(|| format!("{offset:#x}: uninitialized SCC"))
                .and_then(|condition| {
                    if condition {
                        Ok(None)
                    } else {
                        index_of(program, target, offset).map(Some)
                    }
                }),
            Insn::BranchMaskZero { target, register } => {
                if register == 106 && vcc_lane_derived {
                    let live = lanes.iter().find(|lane| lane.error.is_none()).unwrap();
                    check_runtime_region(program, pc, target, live, &sgpr, execution_mask, offset)
                        .and_then(|()| {
                            if sgpr[106] == Some(0) {
                                index_of(program, target, offset).map(Some)
                            } else {
                                Ok(None)
                            }
                        })
                } else {
                    sgpr[register]
                        .ok_or_else(|| format!("{offset:#x}: uninitialized mask"))
                        .and_then(|mask| {
                            if mask == 0 {
                                index_of(program, target, offset).map(Some)
                            } else {
                                Ok(None)
                            }
                        })
                }
            }
            Insn::Branch(target) => index_of(program, target, offset).map(Some),
            _ => {
                for (lane_index, lane) in lanes.iter_mut().enumerate() {
                    if lane.error.is_none()
                        && let Err(error) =
                            lane_step(lane, &sgpr, execution_mask, insn, offset, lane_index)
                    {
                        lane.error = Some(error);
                    }
                }
                Ok(None)
            }
        };
        match scalar {
            Ok(Some(next)) => pc = next,
            Ok(None) => pc += 1,
            Err(error) => return Err(wave_error(&mut lanes, error)),
        }
    }
    Err(first_error(&lanes).unwrap_or_else(|| "stream ended without s_endpgm".to_string()))
}

fn scalar_alu(
    sgpr: &mut [Option<u32>; 128],
    scc: &mut Option<bool>,
    (opcode, dst, a, b): (u32, u8, Src, Src),
    offset: usize,
) -> Result<(), String> {
    if dst < 8 {
        return Err(format!("{offset:#x}: write to a resource SGPR"));
    }
    let read = |src: Src| match src {
        Src::Imm(value) => Ok(value),
        Src::Reg(reg) => {
            sgpr[reg as usize].ok_or_else(|| format!("{offset:#x}: uninitialized s{reg}"))
        }
    };
    let (a, b) = (read(a)?, read(b)?);
    let carry_in = || scc.ok_or_else(|| format!("{offset:#x}: uninitialized SCC"));
    let (value, flag) = match opcode {
        0 => ripple(a, b, false),
        // RDNA 2 ISA guide 12.1: signed overflow from the sign bits,
        // add: S0[31] == S1[31] != D[31]; subtract: S0[31] != S1[31] != D[31].
        2 | 3 => {
            let value = if opcode == 2 {
                ripple(a, b, false).0
            } else {
                ripple(a, !b, true).0
            };
            let sign = |word: u32| word & 0x8000_0000 != 0;
            let same = sign(a) == sign(b);
            let overflow = (if opcode == 2 { same } else { !same }) && sign(a) != sign(value);
            (value, overflow)
        }
        4 => ripple(a, b, carry_in()?),
        // a - b - borrow = a + !b + (1 - borrow); borrow-out = !carry-out.
        1 | 5 => {
            let borrow = opcode == 5 && carry_in()?;
            let (value, carry) = ripple(a, !b, !borrow);
            (value, !carry)
        }
        // RDNA 2 ISA guide 12.1: the shifts use only S1[4:0], taken here as
        // the remainder modulo 32, and scale by a power of two in exact
        // arithmetic; ASHR is floor division of the signed S0. SCC = D != 0.
        30 | 32 | 34 => {
            let scale = 1_i64 << (b % 32);
            let value = match opcode {
                30 => (i64::from(a) * scale).rem_euclid(1 << 32),
                32 => i64::from(a) / scale,
                _ => i64::from(a as i32).div_euclid(scale).rem_euclid(1 << 32),
            } as u32;
            (value, value != 0)
        }
        _ => {
            let mut value = 0;
            for bit in 0..32 {
                let (x, y) = ((a >> bit) & 1 == 1, (b >> bit) & 1 == 1);
                let set = match opcode {
                    14 => x && y,
                    16 => x || y,
                    18 => x != y,
                    20 => x && !y,
                    22 => x || !y,
                    24 => !(x && y),
                    26 => !(x || y),
                    _ => x == y,
                };
                value |= u32::from(set) << bit;
            }
            (value, value != 0)
        }
    };
    sgpr[dst as usize] = Some(value);
    *scc = Some(flag);
    Ok(())
}

/// RDNA 2 ISA guide 12.4: SCC = (S0 <relation> S1). Signed operands (opcodes
/// 0-5) are ordered by flipping both sign bits and comparing unsigned.
fn scalar_compare(opcode: u32, a: u32, b: u32) -> bool {
    let (a, b) = if opcode < 6 {
        (a ^ 0x8000_0000, b ^ 0x8000_0000)
    } else {
        (a, b)
    };
    match opcode % 6 {
        0 => a == b,
        1 => a != b,
        2 => a > b,
        3 => a >= b,
        4 => a < b,
        _ => a <= b,
    }
}

fn read_vgpr(lane: &Lane, reg: u8, offset: usize) -> Result<u32, String> {
    if reg == 0 {
        return Err(format!("{offset:#x}: v0 is the resource index"));
    }
    if lane.pending_read == Some(reg) {
        return Err(format!("{offset:#x}: read before load wait"));
    }
    lane.vgpr[reg as usize].ok_or_else(|| format!("{offset:#x}: uninitialized v{reg}"))
}

/// Evaluate one VOPC compare for the whole wave and return the new VCC_LO.
fn compare(
    lanes: &[Lane],
    sgpr: &[Option<u32>; 128],
    execution_mask: u32,
    (relation, signed, src0, src1): (u8, bool, CmpSrc, u8),
    offset: usize,
) -> Result<u32, String> {
    if execution_mask == 0 {
        // Encoding already validated; nothing is read (contract 5).
        return Ok(0);
    }
    let scalar_src0 = match src0 {
        CmpSrc::Sgpr(reg) => {
            Some(sgpr[reg as usize].ok_or_else(|| format!("{offset:#x}: uninitialized s{reg}"))?)
        }
        CmpSrc::Vcc => Some(sgpr[106].ok_or_else(|| format!("{offset:#x}: uninitialized vcc_lo"))?),
        CmpSrc::Imm(value) => Some(value),
        CmpSrc::Vgpr(_) => None,
    };
    // VGPR readiness is wave-uniform; validate it on every existing lane so
    // acceptance does not depend on which lanes are active.
    for lane in lanes {
        if let CmpSrc::Vgpr(reg) = src0 {
            read_vgpr(lane, reg, offset)?;
        }
        read_vgpr(lane, src1, offset)?;
    }
    let mut mask = 0u32;
    for (n, lane) in lanes.iter().enumerate() {
        if (execution_mask >> n) & 1 == 0 {
            continue;
        }
        let a = match (scalar_src0, src0) {
            (Some(value), _) => value,
            (None, CmpSrc::Vgpr(reg)) => read_vgpr(lane, reg, offset)?,
            (None, _) => unreachable!(),
        };
        let b = read_vgpr(lane, src1, offset)?;
        // Signed order equals unsigned order after flipping the sign bit.
        let (a, b) = if signed {
            (a ^ 0x8000_0000, b ^ 0x8000_0000)
        } else {
            (a, b)
        };
        let pass = match relation {
            1 => a < b,
            2 => a == b,
            3 => a <= b,
            4 => a > b,
            5 => a != b,
            _ => a >= b,
        };
        mask |= u32::from(pass) << n;
    }
    Ok(mask)
}

/// Static acceptance of the region skipped by a runtime `s_cbranch_vccz`
/// (contract 6), independent of lane data, EXEC and the branch outcome.
fn check_runtime_region(
    program: &[(usize, Insn)],
    pc: usize,
    target: usize,
    lane: &Lane,
    sgpr: &[Option<u32>; 128],
    execution_mask: u32,
    offset: usize,
) -> Result<(), String> {
    // Contract 6 forbids an outstanding load; a pending store is waited for later.
    if lane.pending_read.is_some() {
        return Err(format!("{offset:#x}: load outstanding at a runtime branch"));
    }
    let end = index_of(program, target, offset)?;
    for &(at, insn) in &program[pc + 1..end] {
        // (destination, VGPR sources, SGPR source)
        let (dst, vgprs, scalar): (u8, Vec<u8>, Option<u8>) = match insn {
            Insn::VMov(dst, Src::Reg(reg)) => (dst, vec![reg], None),
            Insn::VMov(dst, Src::Imm(_)) => (dst, vec![], None),
            Insn::VMovFromSgpr(dst, src) => (dst, vec![], Some(src)),
            Insn::VMovFromVcc(dst) => (dst, vec![], Some(106)),
            Insn::VAddNc(dst, a, b)
            | Insn::VAnd(dst, a, b)
            | Insn::VOr(dst, a, b)
            | Insn::VXor(dst, a, b)
            | Insn::VShiftLeft(dst, a, b)
            | Insn::VShiftRight(dst, a, b)
            | Insn::VShiftRightArithmetic(dst, a, b) => (dst, vec![a, b], None),
            _ => {
                return Err(format!(
                    "{at:#x}: instruction not allowed in a runtime branch region"
                ));
            }
        };
        if dst == 0 || lane.vgpr[dst as usize].is_none() {
            return Err(format!(
                "{at:#x}: runtime branch region writes an uninitialized VGPR"
            ));
        }
        // Sources are read only when some lane executes (contract 6).
        if execution_mask == 0 {
            continue;
        }
        for reg in vgprs {
            read_vgpr(lane, reg, at)?;
        }
        if let Some(reg) = scalar {
            sgpr[reg as usize].ok_or_else(|| format!("{at:#x}: uninitialized s{reg}"))?;
        }
    }
    Ok(())
}

/// One per-lane vector, memory or wait instruction; behaviour and errors match
/// the former independent per-lane interpreter.
fn lane_step(
    lane: &mut Lane,
    sgpr: &[Option<u32>; 128],
    execution_mask: u32,
    insn: Insn,
    offset: usize,
    lane_index: usize,
) -> Result<(), String> {
    let read_s = |reg: u8| -> Result<u32, String> {
        sgpr[reg as usize].ok_or_else(|| format!("{offset:#x}: uninitialized s{reg}"))
    };
    let read_vcc = || sgpr[106].ok_or_else(|| format!("{offset:#x}: uninitialized vcc_lo"));
    if execution_mask.count_ones() < 32 {
        match insn {
            Insn::Load(_) | Insn::Store(_) => {
                return Err(format!(
                    "{offset:#x}: incomplete EXEC on a buffer instruction"
                ));
            }
            Insn::VMov(dst, _)
            | Insn::VMovFromSgpr(dst, _)
            | Insn::VMovFromVcc(dst)
            | Insn::VAddNc(dst, ..)
            | Insn::VAnd(dst, ..)
            | Insn::VOr(dst, ..)
            | Insn::VXor(dst, ..)
            | Insn::VShiftLeft(dst, ..)
            | Insn::VShiftRight(dst, ..)
            | Insn::VShiftRightArithmetic(dst, ..)
            | Insn::VAddF(dst, ..)
            | Insn::VMulF(dst, ..) => {
                if execution_mask != 0 && lane.vgpr[usize::from(dst)].is_none() {
                    return Err(format!("{offset:#x}: partial EXEC on uninitialized VGPR"));
                }
                if execution_mask != 0 {
                    match insn {
                        Insn::VMovFromSgpr(_, src) => {
                            read_s(src)?;
                        }
                        Insn::VMovFromVcc(_) => {
                            read_vcc()?;
                        }
                        _ => {}
                    }
                }
                if (execution_mask.rotate_right(lane_index as u32) & 1) == 0 {
                    return Ok(());
                }
            }
            _ => {}
        }
    }

    let read_v = |reg: u8| read_vgpr(lane, reg, offset);
    let pending_read = lane.pending_read;
    let value = match insn {
        Insn::Load(data) => {
            if data == 0 || pending_read.is_some() {
                return Err(format!("{offset:#x}: invalid or overlapping load"));
            }
            lane.pending_read = Some(data);
            return Ok(());
        }
        Insn::Store(data) => {
            let value = read_v(data)?;
            if lane.pending_write.is_some() || lane.written.is_some() {
                return Err(format!("{offset:#x}: multiple output writes"));
            }
            lane.pending_write = Some(value);
            return Ok(());
        }
        Insn::WaitRead => {
            if let Some(dst) = lane.pending_read.take() {
                lane.vgpr[dst as usize] = Some(lane.input);
            }
            return Ok(());
        }
        Insn::WaitWrite => {
            if let Some(value) = lane.pending_write.take() {
                lane.written = Some(value);
            }
            return Ok(());
        }
        Insn::VMov(dst, src) => {
            let value = match src {
                Src::Imm(value) => value,
                Src::Reg(reg) => read_v(reg)?,
            };
            check_destination(dst, pending_read, offset)?;
            (dst, value)
        }
        Insn::VMovFromSgpr(dst, src) => {
            let value = read_s(src)?;
            check_destination(dst, pending_read, offset)?;
            (dst, value)
        }
        Insn::VMovFromVcc(dst) => {
            let value = read_vcc()?;
            check_destination(dst, pending_read, offset)?;
            (dst, value)
        }
        Insn::VAddNc(dst, a, b) => {
            check_destination(dst, pending_read, offset)?;
            let sum = u64::from(read_v(a)?) + u64::from(read_v(b)?);
            (dst, sum as u32)
        }
        Insn::VAnd(dst, a, b) => {
            check_destination(dst, pending_read, offset)?;
            (dst, read_v(a)? & read_v(b)?)
        }
        Insn::VOr(dst, a, b) => {
            check_destination(dst, pending_read, offset)?;
            (dst, read_v(a)? | read_v(b)?)
        }
        Insn::VShiftLeft(dst, count, value) => {
            check_destination(dst, pending_read, offset)?;
            let factor = 2_u64.pow(read_v(count)? % 32);
            let product = u64::from(read_v(value)?) * factor;
            (dst, (product % (1_u64 << 32)) as u32)
        }
        Insn::VShiftRight(dst, count, value) => {
            check_destination(dst, pending_read, offset)?;
            let divisor = 2_u32.pow(read_v(count)? % 32);
            (dst, read_v(value)? / divisor)
        }
        Insn::VShiftRightArithmetic(dst, count, value) => {
            check_destination(dst, pending_read, offset)?;
            // Floor division of the sign-extended value, reduced modulo
            // 2^32. This is arithmetic shift without using `>>`.
            let shifted =
                i64::from(read_v(value)? as i32).div_euclid(1_i64 << (read_v(count)? % 32));
            (dst, shifted.rem_euclid(1_i64 << 32) as u32)
        }
        Insn::VXor(dst, a, b) => {
            check_destination(dst, pending_read, offset)?;
            (dst, read_v(a)? ^ read_v(b)?)
        }
        Insn::VAddF(dst, a, b) => {
            check_destination(dst, pending_read, offset)?;
            (dst, exact_float(true, read_v(a)?, read_v(b)?, offset)?)
        }
        Insn::VMulF(dst, a, b) => {
            check_destination(dst, pending_read, offset)?;
            (dst, exact_float(false, read_v(a)?, read_v(b)?, offset)?)
        }
        _ => unreachable!("scalar instruction in lane_step"),
    };
    lane.vgpr[value.0 as usize] = Some(value.1);
    Ok(())
}

/// Bit-serial addition with carry-in; returns the sum and the carry-out.
fn ripple(a: u32, b: u32, carry_in: bool) -> (u32, bool) {
    let mut carry = carry_in;
    let mut sum = 0;
    for bit in 0..32 {
        let (x, y) = ((a >> bit) & 1 == 1, (b >> bit) & 1 == 1);
        sum |= u32::from(x ^ y ^ carry) << bit;
        carry = (x && y) || (carry && (x ^ y));
    }
    (sum, carry)
}

fn check_destination(dst: u8, load: Option<u8>, offset: usize) -> Result<(), String> {
    if dst == 0 || load == Some(dst) {
        return Err(format!("{offset:#x}: write to the index or pending VGPR"));
    }
    Ok(())
}

fn index_of(program: &[(usize, Insn)], target: usize, offset: usize) -> Result<usize, String> {
    program
        .iter()
        .position(|(at, _)| *at == target)
        .ok_or_else(|| format!("{offset:#x}: branch target {target:#x} is not a boundary"))
}

fn exact_float(add: bool, a_bits: u32, b_bits: u32, offset: usize) -> Result<u32, String> {
    fn unpack(bits: u32) -> Option<(u64, i32)> {
        let biased = ((bits >> 23) & 0xff) as i32;
        if bits >> 31 != 0 || !(1..=254).contains(&biased) {
            return None;
        }
        let significand = (1 << 23) | (bits & 0x007f_ffff);
        Some((u64::from(significand), biased - 127 - 23))
    }

    let operand_error = || format!("{offset:#x}: float outside positive normal domain");
    let (a, scale_a) = unpack(a_bits).ok_or_else(operand_error)?;
    let (b, scale_b) = unpack(b_bits).ok_or_else(operand_error)?;
    let result_error = || format!("{offset:#x}: float result not exact normal binary32");
    let (integer, scale) = if add {
        let scale = scale_a.min(scale_b);
        // A nonzero operand at least 24 exponents lower is strictly
        // less than one ULP of the larger operand and cannot sum exactly.
        if scale_a.abs_diff(scale_b) >= 24 {
            return Err(result_error());
        }
        ((a << (scale_a - scale)) + (b << (scale_b - scale)), scale)
    } else {
        // Two 24-bit significands multiply exactly in an unsigned 64-bit word.
        (a * b, scale_a + scale_b)
    };

    let leading = integer.ilog2();
    let trailing = integer.trailing_zeros();
    let exponent = scale + leading as i32;
    let significant_width = leading - trailing + 1;
    if significant_width > 24 || !(-126..=127).contains(&exponent) {
        return Err(result_error());
    }
    let normalized = ((integer >> trailing) << (24 - significant_width)) as u32;
    Ok((((exponent + 127) as u32) << 23) | (normalized & 0x007f_ffff))
}

fn decode(code: &[u8]) -> Result<Vec<(usize, Insn)>, String> {
    let mut program = Vec::new();
    let mut offset = 0;
    while offset < code.len() {
        let (insn, len) = decode_one(code, offset)?;
        if matches!(insn, Insn::End) && offset + len != code.len() {
            return Err(format!("{offset:#x}: code after s_endpgm"));
        }
        program.push((offset, insn));
        offset += len;
    }
    for &(at, insn) in &program {
        if let Insn::Branch(target) | Insn::BranchScc0(target) | Insn::BranchMaskZero { target, .. } =
            insn
            && !program.iter().any(|(boundary, _)| *boundary == target)
        {
            return Err(format!(
                "{at:#x}: branch target {target:#x} is not a boundary"
            ));
        }
    }
    Ok(program)
}

fn decode_one(code: &[u8], offset: usize) -> Result<(Insn, usize), String> {
    let first = word(code, offset)?;
    let fail = |detail: &str| format!("{offset:#x}: {detail}");

    // The synthetic ABI permits only idxen with zero offsets. Only the
    // data VGPR varies across each complete eight-byte encoding.
    if matches!(first, 0xe030_2000 | 0xe070_2000) {
        let load = first == 0xe030_2000;
        let expected = if load { 0x8000_0000 } else { 0x8001_0000 };
        let second = word(code, offset + 4)?;
        if second & 0xffff_00ff != expected {
            return Err(fail("buffer operand outside synthetic ABI"));
        }
        let data = (second >> 8) as u8;
        return Ok((
            if load {
                Insn::Load(data)
            } else {
                Insn::Store(data)
            },
            8,
        ));
    }
    if first & 0xfe01_fe00 == 0x7e00_0200 {
        let dst = (first >> 17) as u8;
        let selector = first & 0x1ff;
        if selector <= 105 {
            return Ok((Insn::VMovFromSgpr(dst, selector as u8), 4));
        }
        if selector == 106 {
            return Ok((Insn::VMovFromVcc(dst), 4));
        }
        let (src, len) = if selector == 255 {
            (Src::Imm(word(code, offset + 4)?), 8)
        } else {
            (
                vector_source(selector).ok_or_else(|| fail("bad VOP1 source"))?,
                4,
            )
        };
        return Ok((Insn::VMov(dst, src), len));
    }
    // VOPC: bits 31:25 = 0b0111110, OP 24:17, VSRC1 16:9, SRC0 8:0.
    if first >> 25 == 0b011_1110 {
        let opcode = (first >> 17) & 0xff;
        let (signed, relation) = match opcode {
            129..=134 => (true, opcode - 128),
            193..=198 => (false, opcode - 192),
            _ => return Err(fail("unsupported VOPC opcode")),
        };
        let selector = first & 0x1ff;
        let (src0, len) = match selector {
            0..=105 => (CmpSrc::Sgpr(selector as u8), 4),
            106 => (CmpSrc::Vcc, 4),
            128..=192 => (CmpSrc::Imm(selector - 128), 4),
            193..=208 => (CmpSrc::Imm(192u32.wrapping_sub(selector)), 4),
            255 => (
                CmpSrc::Imm(
                    word(code, offset + 4)
                        .map_err(|_| format!("{offset:#x}: truncated VOPC literal"))?,
                ),
                8,
            ),
            256..=511 => (CmpSrc::Vgpr(selector as u8), 4),
            _ => return Err(fail("bad VOPC source")),
        };
        let insn = Insn::VCmp {
            relation: relation as u8,
            signed,
            src0,
            src1: (first >> 9) as u8,
        };
        return Ok((insn, len));
    }
    if first & 0xffff_0000 == 0xd525_0000 {
        let second = word(code, offset + 4)?;
        let src0 = second & 0x1ff;
        let src1 = (second >> 9) & 0x1ff;
        if first & 0xff00 != 0 || second >> 18 != 128 || src0 < 256 || src1 < 256 {
            return Err(fail("bad VOP3 add form"));
        }
        return Ok((Insn::VAddNc(first as u8, src0 as u8, src1 as u8), 8));
    }
    if first & 0xff80_ff00 == 0xbe80_0300 {
        let dst = ((first >> 16) & 0x7f) as u8;
        if dst > 107 && dst < 126 {
            return Err(fail("unsupported SOP1 destination"));
        }
        let (src, len) = scalar_source(code, offset, first as u8)?;
        return Ok((Insn::SMov(dst, src), len));
    }
    // SOP2: ENCODING 0b10 in bits 30-31, OP in bits 23-29, SDST in 16-22.
    if first >> 30 == 0b10
        && matches!(
            (first >> 23) & 0x7f,
            0..=5 | 14 | 16 | 18 | 20 | 22 | 24 | 26 | 28 | 30 | 32 | 34
        )
    {
        let dst = ((first >> 16) & 0x7f) as u8;
        if dst > 105 {
            return Err(fail("unsupported SOP2 destination"));
        }
        let (a, b) = (first as u8, (first >> 8) as u8);
        if a == 255 && b == 255 {
            return Err(fail("two scalar literals"));
        }
        let (sa, la) = scalar_source(code, offset, a)?;
        let (sb, lb) = scalar_source(code, offset, b)?;
        return Ok((Insn::SAlu((first >> 23) & 0x7f, dst, sa, sb), la.max(lb)));
    }
    // SOPC: ENCODING 0b10_1111110 in bits 23-31, OP in 16-22.
    if first >> 23 == 0b1_0111_1110 && (first >> 16) & 0x7f <= 11 {
        let a = first as u8;
        let b = (first >> 8) as u8;
        if a == 255 && b == 255 {
            return Err(fail("two scalar literals"));
        }
        let (sa, la) = scalar_source(code, offset, a)?;
        let (sb, lb) = scalar_source(code, offset, b)?;
        return Ok((Insn::SCmp((first >> 16) & 0x7f, sa, sb), la.max(lb)));
    }
    if first & 0xffff_0000 == 0xbf82_0000 {
        return Ok((Insn::Branch(branch_target(first, offset)?), 4));
    }
    if first & 0xffff_0000 == 0xbf84_0000 {
        return Ok((Insn::BranchScc0(branch_target(first, offset)?), 4));
    }
    let tested_register = match first >> 16 {
        0xbf86 => Some(106),
        0xbf88 => Some(126),
        _ => None,
    };
    if let Some(register) = tested_register {
        return Ok((
            Insn::BranchMaskZero {
                target: branch_target(first, offset)?,
                register,
            },
            4,
        ));
    }
    for (mask, value, kind) in [
        (0xfe00_0000, 0x3600_0000u32, 0u8),
        (0xfe00_0000, 0x3a00_0000, 1),
        (0xfe00_0000, 0x0600_0000, 2),
        (0xfe00_0000, 0x1000_0000, 3),
        (0xfe00_0000, 0x3800_0000, 4),
        (0xfe00_0000, 26 << 25, 5),
        (0xfe00_0000, 22 << 25, 6),
        (0xfe00_0000, 24 << 25, 7),
    ] {
        if first & mask == value {
            if first & 0x100 == 0 {
                return Err(fail("VOP2 source modifier"));
            }
            let dst = (first >> 17) as u8;
            let a = first as u8;
            let b = (first >> 9) as u8;
            let insn = match kind {
                0 => Insn::VAnd(dst, a, b),
                1 => Insn::VXor(dst, a, b),
                2 => Insn::VAddF(dst, a, b),
                3 => Insn::VMulF(dst, a, b),
                4 => Insn::VOr(dst, a, b),
                5 => Insn::VShiftLeft(dst, a, b),
                6 => Insn::VShiftRight(dst, a, b),
                _ => Insn::VShiftRightArithmetic(dst, a, b),
            };
            return Ok((insn, 4));
        }
    }
    if first & 0xffff_0000 == 0xbf8c_0000 {
        if first != 0xbf8c_3f70 {
            return Err(fail("waitcnt not vmcnt(0)"));
        }
        return Ok((Insn::WaitRead, 4));
    }
    if first & 0xff80_0000 == 0xbb80_0000 {
        if first != 0xbbfd_0000 {
            return Err(fail("waitcnt_vscnt not null,0"));
        }
        return Ok((Insn::WaitWrite, 4));
    }
    if first & 0xffff_0000 == 0xbf81_0000 {
        if first != 0xbf81_0000 {
            return Err(fail("endpgm immediate"));
        }
        return Ok((Insn::End, 4));
    }
    Err(fail("unsupported opcode"))
}

fn vector_source(selector: u32) -> Option<Src> {
    match selector {
        128..=192 => Some(Src::Imm(selector - 128)),
        193..=208 => Some(Src::Imm(192u32.wrapping_sub(selector))),
        242 => Some(Src::Imm(0x3f80_0000)),
        256..=511 => Some(Src::Reg(selector as u8)),
        _ => None,
    }
}

fn scalar_source(code: &[u8], offset: usize, selector: u8) -> Result<(Src, usize), String> {
    match selector {
        0..=105 => Ok((Src::Reg(selector), 4)),
        128..=192 => Ok((Src::Imm(u32::from(selector - 128)), 4)),
        193..=208 => Ok((Src::Imm(192u32.wrapping_sub(u32::from(selector))), 4)),
        255 => Ok((Src::Imm(word(code, offset + 4)?), 8)),
        _ => Err(format!("{offset:#x}: bad scalar source")),
    }
}

fn branch_target(first: u32, offset: usize) -> Result<usize, String> {
    let displacement = first as u16 as i16;
    let words =
        usize::try_from(displacement).map_err(|_| format!("{offset:#x}: backward branch"))?;
    Ok(offset + 4 + words * 4)
}

fn word(code: &[u8], offset: usize) -> Result<u32, String> {
    let bytes = code
        .get(offset..offset + 4)
        .ok_or_else(|| format!("{offset:#x}: truncated instruction"))?;
    Ok(u32::from_le_bytes(bytes.try_into().unwrap()))
}
