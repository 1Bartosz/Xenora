// SPDX-License-Identifier: GPL-3.0-or-later

//! Independent CPU execution of the synthetic gfx1030 fixture subset, reference sampling, and host-side references for depth-tested rendering, indexed draws and blending.

pub mod blend;
pub mod depth;
pub mod indexed;
pub mod texture_sampling;

use std::fmt;

const MAX_CODE_BYTES: usize = 1024;
const MAX_INPUT_WORDS: usize = 4096;

#[derive(Debug, PartialEq, Eq)]
pub struct ReferenceError {
    pub offset: usize,
    pub detail: &'static str,
}

impl fmt::Display for ReferenceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "reference byte {:#x}: {}", self.offset, self.detail)
    }
}

impl std::error::Error for ReferenceError {}

/// Register index or immediate bits. The opcode determines the register bank.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Source {
    Register(u8),
    Immediate(u32),
}

/// Operations decoded independently by the reference interpreter.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Op {
    Load {
        data: u8,
        index: u8,
        resource: u8,
    },
    Store {
        data: u8,
        index: u8,
        resource: u8,
    },
    WaitRead,
    WaitWrite,
    Move(u8, Source),
    Add(u8, u8, u8),
    ScalarMove(u8, Source),
    /// VOP1 move of a normal SGPR or `vcc_lo` (106) into a VGPR.
    MoveScalar(u8, u8),
    /// VOPC 32-bit integer compare writing the Wave32 mask to `vcc_lo`.
    Compare(Comparison, CompareSource, u8),
    /// SOP2 operation writing an SGPR and SCC.
    Scalar(ScalarAlu, u8, Source, Source),
    CompareEqual(Source, Source),
    Branch {
        target: usize,
        condition: BranchCondition,
    },
    Vector(VectorAlu, u8, u8, u8),
    End,
}

/// Relation and operand interpretation of a VOPC integer compare.
///
/// AMD RDNA 2 ISA guide section 12.9, Tables 59 and 61: opcode 128 + offset
/// for signed and 192 + offset for unsigned 32-bit operands, with offsets
/// LT 1, EQ 2, LE 3, GT 4, NE 5, GE 6.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Comparison {
    /// Opcode offset 1-6 within the 32-bit integer group.
    pub offset: u8,
    /// Whether the operands are two's-complement integers.
    pub signed: bool,
}

impl Comparison {
    /// The per-lane result, from an exact comparison of the widened operands.
    fn holds(self, s0: u32, s1: u32) -> bool {
        let widen = |bits: u32| {
            if self.signed {
                i64::from(bits as i32)
            } else {
                i64::from(bits)
            }
        };
        let (s0, s1) = (widen(s0), widen(s1));
        match self.offset {
            1 => s0 < s1,
            2 => s0 == s1,
            3 => s0 <= s1,
            4 => s0 > s1,
            5 => s0 != s1,
            _ => s0 >= s1,
        }
    }
}

/// First source of a VOPC compare.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CompareSource {
    /// VGPR index.
    Vector(u8),
    /// SGPR index s0-s105, or 106 for `vcc_lo`.
    Scalar(u8),
    /// Inline integer or literal bits.
    Immediate(u32),
}

/// Scalar condition tested by a forward branch.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BranchCondition {
    /// Take the branch unconditionally.
    Always,
    /// Take the branch when SCC is clear.
    SccZero,
    /// Take the branch when the low half of VCC is clear.
    VccZero,
    /// Take the branch when the low half of EXEC is clear.
    ExecZero,
}

/// SOP2 operations decoded by the reference interpreter.
///
/// AMD RDNA 2 XML descriptions: additions store the carry-out in SCC,
/// subtractions the borrow; ADDC/SUBB add or subtract SCC; bitwise
/// operations set SCC iff the result is nonzero.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScalarAlu {
    Add,
    Subtract,
    AddWithCarry,
    SubtractWithBorrow,
    /// SCC = signed overflow (RDNA 2 ISA guide 12.1).
    AddSigned,
    SubtractSigned,
    And,
    Or,
    Xor,
}

/// VOP2 operations decoded by the reference interpreter.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VectorAlu {
    ShiftLeft,
    ShiftRight,
    ArithmeticShiftRight,
    And,
    Or,
    Xor,
    FloatAdd,
    FloatMultiply,
}

/// Execute original little-endian gfx1030 bytes with the synthetic binding ABI.
///
/// Each dispatched lane reads the corresponding input word and may write only
/// its own output word. Integer operations are exact modulo 2^32.
pub fn execute(code: &[u8], input: &[u32]) -> Result<Vec<u32>, ReferenceError> {
    if input.is_empty() || input.len() > MAX_INPUT_WORDS {
        return Err(error(0, "input word count outside reference limits"));
    }
    if code.len() > MAX_CODE_BYTES {
        return Err(error(0, "instruction stream exceeds reference limit"));
    }
    let program = decode(code)?;
    for instruction in &program {
        match instruction.op {
            Op::End if instruction.offset + instruction.byte_len != code.len() => {
                return Err(error(
                    instruction.offset + instruction.byte_len,
                    "trailing code after s_endpgm",
                ));
            }
            Op::Load {
                index, resource, ..
            } if (index, resource) != (0, 0) => {
                return Err(error(
                    instruction.offset,
                    "load outside synthetic binding ABI",
                ));
            }
            Op::Store {
                index, resource, ..
            } if (index, resource) != (0, 4) => {
                return Err(error(
                    instruction.offset,
                    "store outside synthetic binding ABI",
                ));
            }
            _ => {}
        }
    }
    if !program
        .last()
        .is_some_and(|instruction| instruction.op == Op::End)
    {
        return Err(error(code.len(), "missing s_endpgm"));
    }
    // Waves are consecutive groups of 32 dispatch indices; a final partial
    // wave has absent lanes, which contribute 0 to compare masks.
    let mut output = Vec::with_capacity(input.len());
    for wave in input.chunks(32) {
        output.extend(execute_wave(&program, wave)?);
    }
    Ok(output)
}

fn error(offset: usize, detail: &'static str) -> ReferenceError {
    ReferenceError { offset, detail }
}

/// An operation and its span in the original byte stream.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DecodedInstruction {
    pub offset: usize,
    pub byte_len: usize,
    pub op: Op,
}

/// Decode the supported gfx1030 subset without applying the execution ABI.
///
/// AMD RDNA 2 ISA guide, Instruction Encoding. Identify the forms by bytes
/// and independently extract their variable fields.
///
/// # Errors
///
/// Returns the failing byte offset for an unsupported, truncated
/// or invalid instruction stream.
pub fn decode(code: &[u8]) -> Result<Vec<DecodedInstruction>, ReferenceError> {
    let mut program = Vec::new();
    let mut offset = 0;
    while let Some(bytes) = code.get(offset..).filter(|bytes| !bytes.is_empty()) {
        let (op, length) = match bytes {
            [
                0,
                0x20,
                opcode @ (0x30 | 0x70),
                0xe0,
                index,
                data,
                group @ 0..=25,
                0x80,
                ..,
            ] => {
                let (data, index, resource) = (*data, *index, *group * 4);
                let op = if *opcode == 0x30 {
                    Op::Load {
                        data,
                        index,
                        resource,
                    }
                } else {
                    Op::Store {
                        data,
                        index,
                        resource,
                    }
                };
                (op, 8)
            }
            [0x70, 0x3f, 0x8c, 0xbf, ..] => (Op::WaitRead, 4),
            [0x00, 0x00, 0xfd, 0xbb, ..] => (Op::WaitWrite, 4),
            [0x00, 0x00, 0x81, 0xbf, ..] => (Op::End, 4),
            [
                source,
                3,
                destination @ (0x80..=0xeb | 0xfe..=0xff),
                0xbe,
                tail @ ..,
            ] => {
                let (source, length) = decode_scalar(*source, tail, offset)?;
                (Op::ScalarMove(destination & 0x7f, source), length)
            }
            [left, right, 6, 0xbf, tail @ ..] if (*left, *right) != (255, 255) => {
                let (left, left_length) = decode_scalar(*left, tail, offset)?;
                let (right, right_length) = decode_scalar(*right, tail, offset)?;
                (Op::CompareEqual(left, right), left_length.max(right_length))
            }
            // SOP2: OP in bits 23-29, SDST in bits 16-22, SSRC1, SSRC0.
            [
                left,
                right,
                low,
                high @ (0x80..=0x82 | 0x87 | 0x88 | 0x89),
                tail @ ..,
            ] if (*left, *right) != (255, 255)
                && low & 0x7f <= 105
                && (low & 0x80 == 0 || matches!(high, 0x80..=0x82)) =>
            {
                let kind = match (high, low >> 7) {
                    (0x80, 0) => ScalarAlu::Add,
                    (0x80, _) => ScalarAlu::Subtract,
                    (0x81, 0) => ScalarAlu::AddSigned,
                    (0x81, _) => ScalarAlu::SubtractSigned,
                    (0x82, 0) => ScalarAlu::AddWithCarry,
                    (0x82, _) => ScalarAlu::SubtractWithBorrow,
                    (0x87, _) => ScalarAlu::And,
                    (0x88, _) => ScalarAlu::Or,
                    _ => ScalarAlu::Xor,
                };
                let (left, left_length) = decode_scalar(*left, tail, offset)?;
                let (right, right_length) = decode_scalar(*right, tail, offset)?;
                (
                    Op::Scalar(kind, low & 0x7f, left, right),
                    left_length.max(right_length),
                )
            }
            [lo, hi, opcode @ (0x82 | 0x84 | 0x86 | 0x88), 0xbf, ..] => {
                let words = i16::from_le_bytes([*lo, *hi]);
                let words = usize::try_from(words).map_err(|_| error(offset, "backward branch"))?;
                (
                    Op::Branch {
                        target: offset + 4 + words * 4,
                        condition: match opcode {
                            0x84 => BranchCondition::SccZero,
                            0x86 => BranchCondition::VccZero,
                            0x88 => BranchCondition::ExecZero,
                            _ => BranchCondition::Always,
                        },
                    },
                    4,
                )
            }
            [first, second, third, opcode, ..]
                if matches!(
                    opcode & 0xfe,
                    0x06 | 0x10 | 0x2c | 0x30 | 0x34 | 0x36 | 0x38 | 0x3a
                ) && second & 1 != 0 =>
            {
                let kind = match opcode & 0xfe {
                    0x06 => VectorAlu::FloatAdd,
                    0x10 => VectorAlu::FloatMultiply,
                    0x34 => VectorAlu::ShiftLeft,
                    0x2c => VectorAlu::ShiftRight,
                    0x30 => VectorAlu::ArithmeticShiftRight,
                    0x36 => VectorAlu::And,
                    0x38 => VectorAlu::Or,
                    _ => VectorAlu::Xor,
                };
                let dst = (third >> 1) | ((opcode & 1) << 7);
                let src1 = (second >> 1) | ((third & 1) << 7);
                (Op::Vector(kind, dst, *first, src1), 4)
            }
            [dst, 0, 0x25, 0xd5, lo, middle, hi @ 2..=3, 0x02, ..] if middle & 1 == 1 => {
                let src1 = (middle >> 1) | ((hi & 1) << 7);
                (Op::Add(*dst, *lo, src1), 8)
            }
            // VOPC (bits 31-25 = 0b0111110): SRC0 bits 0-8, VSRC1 bits 9-16, OP bits 17-24.
            [lo, packed, upper, tag @ 0x7c..=0x7d, tail @ ..]
                if matches!(
                    (u16::from(*tag & 1) << 7) | u16::from(*upper >> 1),
                    129..=134 | 193..=198
                ) =>
            {
                let opcode = (u16::from(*tag & 1) << 7) | u16::from(*upper >> 1);
                let comparison = Comparison {
                    offset: (opcode & 0x3f) as u8,
                    signed: opcode < 192,
                };
                let vsrc1 = (packed >> 1) | ((upper & 1) << 7);
                let selector = u16::from(*lo) | (u16::from(packed & 1) << 8);
                let (src0, length) = match selector {
                    0..=106 => (CompareSource::Scalar(*lo), 4),
                    128..=192 => (CompareSource::Immediate(u32::from(selector - 128)), 4),
                    193..=208 => (
                        CompareSource::Immediate(0_u32.wrapping_sub(u32::from(selector - 192))),
                        4,
                    ),
                    255 => {
                        let literal = tail
                            .first_chunk::<4>()
                            .ok_or_else(|| error(offset, "truncated literal"))?;
                        (CompareSource::Immediate(u32::from_le_bytes(*literal)), 8)
                    }
                    256..=511 => (CompareSource::Vector(*lo), 4),
                    _ => return Err(error(offset, "unsupported VOPC source")),
                };
                (Op::Compare(comparison, src0, vsrc1), length)
            }
            [lo, packed, upper, tag @ 0x7e..=0x7f, tail @ ..]
                if packed & 0xfe == 2 && upper & 1 == 0 =>
            {
                let dst = (upper >> 1) | ((tag & 1) << 7);
                let selector = u16::from(*lo) | (u16::from(packed & 1) << 8);
                if selector <= 106 {
                    // VOP1 SRC0 0-105 selects a normal SGPR, 106 vcc_lo.
                    program.push(DecodedInstruction {
                        offset,
                        byte_len: 4,
                        op: Op::MoveScalar(dst, *lo),
                    });
                    offset += 4;
                    continue;
                }
                let (src, length) = match selector {
                    128..=192 => (Source::Immediate(u32::from(selector - 128)), 4),
                    193..=208 => (
                        Source::Immediate(0_u32.wrapping_sub(u32::from(selector - 192))),
                        4,
                    ),
                    242 => (Source::Immediate(1.0f32.to_bits()), 4),
                    255 => {
                        let literal = tail
                            .first_chunk::<4>()
                            .ok_or_else(|| error(offset, "truncated literal"))?;
                        (Source::Immediate(u32::from_le_bytes(*literal)), 8)
                    }
                    256..=511 => (Source::Register(*lo), 4),
                    _ => return Err(error(offset, "unsupported VOP1 source")),
                };
                (Op::Move(dst, src), length)
            }
            _ => {
                return Err(error(
                    offset,
                    "unsupported or truncated gfx1030 instruction",
                ));
            }
        };
        program.push(DecodedInstruction {
            offset,
            byte_len: length,
            op,
        });
        offset += length;
    }
    for instruction in &program {
        if let Op::Branch { target, .. } = instruction.op
            && program
                .binary_search_by_key(&target, |item| item.offset)
                .is_err()
        {
            return Err(error(
                instruction.offset,
                "branch target is not an instruction boundary",
            ));
        }
    }
    Ok(program)
}

fn decode_scalar(
    selector: u8,
    tail: &[u8],
    offset: usize,
) -> Result<(Source, usize), ReferenceError> {
    match selector {
        0..=105 => Ok((Source::Register(selector), 4)),
        128..=192 => Ok((Source::Immediate(u32::from(selector - 128)), 4)),
        193..=208 => Ok((
            Source::Immediate(0u32.wrapping_sub(u32::from(selector - 192))),
            4,
        )),
        255 => {
            let literal = tail
                .first_chunk::<4>()
                .ok_or_else(|| error(offset, "truncated scalar literal"))?;
            Ok((Source::Immediate(u32::from_le_bytes(*literal)), 8))
        }
        _ => Err(error(offset, "unsupported scalar source")),
    }
}

fn vector_alu(op: VectorAlu, a: u32, b: u32, offset: usize) -> Result<u32, ReferenceError> {
    let error = || {
        error(
            offset,
            "float operation outside positive, normal, exact binary32 domain",
        )
    };
    match op {
        VectorAlu::ShiftLeft => return Ok(b << (a & 31)),
        VectorAlu::ShiftRight => return Ok(b >> (a & 31)),
        VectorAlu::ArithmeticShiftRight => return Ok(((b as i32) >> (a & 31)) as u32),
        VectorAlu::And => return Ok(a & b),
        VectorAlu::Or => return Ok(a | b),
        VectorAlu::Xor => return Ok(a ^ b),
        _ => {}
    }
    let (a, b) = (f32::from_bits(a), f32::from_bits(b));
    if !a.is_normal() || !b.is_normal() || a.is_sign_negative() || b.is_sign_negative() {
        return Err(error());
    }
    let (a, b) = (f64::from(a), f64::from(b));
    let exact = match op {
        VectorAlu::FloatAdd => {
            let sum = a + b;
            // Recover the smaller input to detect even a binary64 addition rounding.
            if sum - a.max(b) != a.min(b) {
                return Err(error());
            }
            sum
        }
        VectorAlu::FloatMultiply => a * b, // Two 24-bit significands fit in binary64.
        VectorAlu::ShiftLeft
        | VectorAlu::ShiftRight
        | VectorAlu::ArithmeticShiftRight
        | VectorAlu::And
        | VectorAlu::Or
        | VectorAlu::Xor => unreachable!(),
    };
    let result = exact as f32;
    if !result.is_normal() || f64::from(result) != exact {
        return Err(error());
    }
    Ok(result.to_bits())
}

const VCC_LO: usize = 106;
const VCC_HI: usize = 107;
const EXEC_LO: usize = 126;

/// Runs one Wave32 wave in lockstep. Scalar state is shared by the lanes;
/// VGPRs, transfers and outputs are per lane.
fn execute_wave(program: &[DecodedInstruction], input: &[u32]) -> Result<Vec<u32>, ReferenceError> {
    let lanes = input.len();
    let mut registers = vec![[None::<u32>; 256]; lanes];
    let mut scalars = [None; 128];
    let mut exec = u32::MAX;
    scalars[EXEC_LO] = Some(exec);
    let mut scc = None;
    // Whether VCC_LO was last written by a compare with nonzero EXEC.
    let mut lane_derived_vcc = false;
    let mut target = 0;
    let mut pending_read = None;
    let mut pending_write = vec![None; lanes];
    let mut output = vec![None; lanes];
    for (position, &DecodedInstruction { offset, op, .. }) in program.iter().enumerate() {
        if offset < target {
            continue;
        }
        let read_scalar = |source| match source {
            Source::Register(reg) => scalars[usize::from(reg)]
                .ok_or_else(|| error(offset, "read from uninitialized SGPR")),
            Source::Immediate(value) => Ok(value),
        };
        let read = |lane: usize, register: u8| {
            if register == 0 {
                return Err(error(offset, "v0 is reserved for the resource index"));
            }
            if pending_read == Some(register) {
                return Err(error(offset, "VGPR read before load wait"));
            }
            registers[lane][usize::from(register)]
                .ok_or_else(|| error(offset, "read from uninitialized VGPR"))
        };
        // Lanes that execute a vector ALU instruction under the current EXEC.
        let executes = |lane: usize| exec & (1 << lane) != 0;
        if exec != u32::MAX {
            match op {
                Op::Move(dst, _)
                | Op::MoveScalar(dst, _)
                | Op::Add(dst, ..)
                | Op::Vector(_, dst, ..) => {
                    if exec != 0
                        && registers
                            .iter()
                            .any(|lane| lane[usize::from(dst)].is_none())
                    {
                        return Err(error(offset, "partial EXEC requires an initialized VGPR"));
                    }
                    // As in translation, an SGPR source must be known even for an inactive lane.
                    if let Op::MoveScalar(_, src) = op
                        && exec != 0
                    {
                        read_scalar(Source::Register(src))?;
                    }
                }
                Op::Load { .. } | Op::Store { .. } => {
                    return Err(error(offset, "vector memory requires full Wave32 EXEC"));
                }
                _ => {}
            }
        }
        // Per-lane results of a vector ALU instruction, applied after all lanes read.
        let mut results: Vec<Option<(u8, u32)>> = vec![None; lanes];
        match op {
            Op::Load { data: dst, .. } => {
                if dst == 0 || pending_read.is_some() {
                    return Err(error(
                        offset,
                        "invalid load destination or overlapping load",
                    ));
                }
                pending_read = Some(dst);
            }
            Op::Store { data: src, .. } => {
                for lane in 0..lanes {
                    let value = read(lane, src)?;
                    if pending_write[lane].is_some() || output[lane].is_some() {
                        return Err(error(offset, "multiple writes to the output"));
                    }
                    pending_write[lane] = Some(value);
                }
            }
            Op::WaitRead => {
                if let Some(dst) = pending_read.take() {
                    for (lane, &word) in input.iter().enumerate() {
                        results[lane] = Some((dst, word));
                    }
                }
            }
            Op::WaitWrite => {
                for lane in 0..lanes {
                    if let Some(value) = pending_write[lane].take() {
                        output[lane] = Some(value);
                    }
                }
            }
            Op::Move(dst, source) => {
                for lane in (0..lanes).filter(|&lane| executes(lane)) {
                    let value = match source {
                        Source::Register(src) => read(lane, src)?,
                        Source::Immediate(value) => value,
                    };
                    results[lane] = Some((dst, value));
                }
            }
            Op::Add(dst, lhs, rhs) => {
                for lane in (0..lanes).filter(|&lane| executes(lane)) {
                    let value = read(lane, lhs)?.wrapping_add(read(lane, rhs)?);
                    results[lane] = Some((dst, value));
                }
            }
            Op::MoveScalar(dst, src) => {
                for lane in (0..lanes).filter(|&lane| executes(lane)) {
                    results[lane] = Some((dst, read_scalar(Source::Register(src))?));
                }
            }
            Op::Vector(operator, dst, lhs, rhs) => {
                for lane in (0..lanes).filter(|&lane| executes(lane)) {
                    let value = vector_alu(operator, read(lane, lhs)?, read(lane, rhs)?, offset)?;
                    results[lane] = Some((dst, value));
                }
            }
            Op::ScalarMove(dst, src) => {
                if dst < 8 {
                    return Err(error(offset, "write to a reserved resource SGPR"));
                }
                let value = read_scalar(src)?;
                if usize::from(dst) == EXEC_LO {
                    exec = value;
                }
                if usize::from(dst) == VCC_LO {
                    lane_derived_vcc = false;
                }
                scalars[usize::from(dst)] = Some(value);
            }
            Op::CompareEqual(lhs, rhs) => {
                scc = Some(read_scalar(lhs)? == read_scalar(rhs)?);
            }
            Op::Scalar(kind, dst, lhs, rhs) => {
                if dst < 8 {
                    return Err(error(offset, "write to a reserved resource SGPR"));
                }
                let (lhs, rhs) = (i64::from(read_scalar(lhs)?), i64::from(read_scalar(rhs)?));
                let carry = match kind {
                    ScalarAlu::AddWithCarry | ScalarAlu::SubtractWithBorrow => i64::from(
                        scc.ok_or_else(|| error(offset, "carry-in from uninitialized SCC"))?,
                    ),
                    _ => 0,
                };
                // Exact integer arithmetic, then reduction modulo 2^32.
                let (exact, flag) = match kind {
                    ScalarAlu::Add | ScalarAlu::AddWithCarry => {
                        let sum = lhs + rhs + carry;
                        (sum, sum >= 1 << 32)
                    }
                    ScalarAlu::Subtract | ScalarAlu::SubtractWithBorrow => {
                        let difference = lhs - rhs - carry;
                        (difference, difference < 0)
                    }
                    // The exact signed result must fit in i32, else SCC = overflow.
                    ScalarAlu::AddSigned | ScalarAlu::SubtractSigned => {
                        let signed = |value: i64| i64::from(value as u32 as i32);
                        let exact = if kind == ScalarAlu::AddSigned {
                            signed(lhs) + signed(rhs)
                        } else {
                            signed(lhs) - signed(rhs)
                        };
                        (exact, i32::try_from(exact).is_err())
                    }
                    ScalarAlu::And | ScalarAlu::Or | ScalarAlu::Xor => {
                        let bits = match kind {
                            ScalarAlu::And => lhs & rhs,
                            ScalarAlu::Or => lhs | rhs,
                            _ => lhs ^ rhs,
                        };
                        (bits, bits != 0)
                    }
                };
                let value = exact.rem_euclid(1 << 32) as u32;
                scalars[usize::from(dst)] = Some(value);
                scc = Some(flag);
            }
            // RDNA 2 ISA section 3.9: VCC[n] = EXEC[n] & test(n), fully written.
            Op::Compare(comparison, src0, src1) => {
                let mut mask = 0u32;
                if exec != 0 {
                    // Sources are checked once for the wave, even if no present lane is active.
                    let check = |register: u8| {
                        (0..lanes).try_for_each(|lane| read(lane, register).map(|_| ()))
                    };
                    match src0 {
                        CompareSource::Vector(register) => check(register)?,
                        CompareSource::Scalar(register) => {
                            read_scalar(Source::Register(register))?;
                        }
                        CompareSource::Immediate(_) => {}
                    }
                    check(src1)?;
                    for lane in (0..lanes).filter(|&lane| executes(lane)) {
                        let s0 = match src0 {
                            CompareSource::Vector(register) => read(lane, register)?,
                            CompareSource::Scalar(register) => {
                                read_scalar(Source::Register(register))?
                            }
                            CompareSource::Immediate(bits) => bits,
                        };
                        if comparison.holds(s0, read(lane, src1)?) {
                            mask |= 1 << lane;
                        }
                    }
                }
                scalars[VCC_LO] = Some(mask);
                scalars[VCC_HI] = None;
                lane_derived_vcc = exec != 0;
            }
            Op::Branch {
                target: destination,
                condition,
            } => {
                if condition == BranchCondition::VccZero && scalars[VCC_LO].is_none() {
                    return Err(error(offset, "branch with uninitialized VCC_LO"));
                }
                if condition == BranchCondition::VccZero && lane_derived_vcc {
                    check_runtime_region(
                        &program[position + 1..],
                        destination,
                        &registers,
                        &scalars,
                        exec,
                        pending_read.is_some(),
                        offset,
                    )?;
                }
                let take_branch = match condition {
                    BranchCondition::Always => true,
                    BranchCondition::SccZero => {
                        !scc.ok_or_else(|| error(offset, "branch with uninitialized SCC"))?
                    }
                    BranchCondition::VccZero => scalars[VCC_LO] == Some(0),
                    BranchCondition::ExecZero => exec == 0,
                };
                if take_branch {
                    target = destination;
                }
            }
            Op::End => {
                if pending_read.is_some() || pending_write.iter().any(Option::is_some) {
                    return Err(error(offset, "outstanding transfer at termination"));
                }
                return output
                    .iter()
                    .map(|value| {
                        value.ok_or_else(|| error(offset, "lane did not write its output"))
                    })
                    .collect();
            }
        }
        for (lane, result) in results.into_iter().enumerate() {
            if let Some((dst, value)) = result {
                if dst == 0 || pending_read == Some(dst) {
                    return Err(error(offset, "write to immutable or pending VGPR"));
                }
                registers[lane][usize::from(dst)] = Some(value);
            }
        }
    }
    Err(error(0, "missing decoded termination"))
}

/// docs/shaders/vector-compare.md section 6: code skipped by `s_cbranch_vccz`
/// on a lane-derived VCC_LO may hold only integer vector ALU instructions
/// whose destinations and sources are ready at the branch. Checked on the
/// code itself, whether or not the branch is taken.
fn check_runtime_region(
    following: &[DecodedInstruction],
    end: usize,
    registers: &[[Option<u32>; 256]],
    scalars: &[Option<u32>; 128],
    exec: u32,
    load_pending: bool,
    branch: usize,
) -> Result<(), ReferenceError> {
    if load_pending {
        return Err(error(branch, "runtime VCC branch with an outstanding load"));
    }
    let ready = |register: u8| {
        register != 0
            && registers
                .iter()
                .all(|lane| lane[usize::from(register)].is_some())
    };
    for instruction in following.iter().take_while(|item| item.offset < end) {
        let offset = instruction.offset;
        let (dst, vector_sources, scalar_source) = match instruction.op {
            Op::Move(dst, Source::Register(src)) => (dst, vec![src], None),
            Op::Move(dst, Source::Immediate(_)) => (dst, vec![], None),
            Op::MoveScalar(dst, src) => (dst, vec![], Some(src)),
            Op::Add(dst, lhs, rhs) => (dst, vec![lhs, rhs], None),
            Op::Vector(kind, dst, lhs, rhs)
                if !matches!(kind, VectorAlu::FloatAdd | VectorAlu::FloatMultiply) =>
            {
                (dst, vec![lhs, rhs], None)
            }
            _ => {
                return Err(error(
                    offset,
                    "runtime VCC branch skips a non-integer-vector instruction",
                ));
            }
        };
        if !ready(dst) {
            return Err(error(
                offset,
                "runtime VCC branch skips a write to an uninitialized VGPR",
            ));
        }
        if exec == 0 {
            continue;
        }
        if vector_sources.into_iter().any(|register| !ready(register)) {
            return Err(error(
                offset,
                "runtime VCC branch skips an unready VGPR read",
            ));
        }
        if let Some(register) = scalar_source
            && scalars[usize::from(register)].is_none()
        {
            return Err(error(
                offset,
                "runtime VCC branch skips an unknown SGPR read",
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &[u8] = include_bytes!("../../../fixtures/rdna2/add.bin");

    #[test]
    fn computes_the_independent_known_answers() {
        let input = [0, 1, u32::MAX, 0u32.wrapping_sub(0x1234_5678)];
        let expected = [0x1234_5678, 0x1234_5679, 0x1234_5677, 0];
        assert_eq!(execute(FIXTURE, &input), Ok(expected.to_vec()));
    }

    #[test]
    fn executes_both_scalar_branch_outcomes() {
        let fixture = include_bytes!("../../../fixtures/rdna2/bitwise-branch.bin");
        let inputs = [0, u32::MAX, 0x1234_5678];
        assert_eq!(
            execute(fixture, &inputs),
            Ok(vec![0x1234_5678, 0x12cb_5687, 0x1200_5600])
        );
        let mut equal = *fixture;
        equal[17] = 0x87; // Compare s8 with 7, so the SCC-zero edge is not taken.
        assert_eq!(execute(&equal, &inputs), Ok(vec![0; 3]));
    }

    #[test]
    fn rejects_invalid_branch_even_when_not_taken() {
        let fixture = include_bytes!("../../../fixtures/rdna2/bitwise-branch.bin");
        let mut invalid = *fixture;
        invalid[17] = 0x87;
        invalid[20] = 3; // Target byte 36, the VOP1 literal.
        assert_eq!(
            execute(&invalid, &[0]).unwrap_err().detail,
            "branch target is not an instruction boundary"
        );
        invalid[20..22].copy_from_slice(&(-1i16).to_le_bytes());
        assert_eq!(
            execute(&invalid, &[0]).unwrap_err().detail,
            "backward branch"
        );
    }

    #[test]
    fn executes_exact_float_fixture_and_rejects_subnormal_operand() {
        let fixture = include_bytes!("../../../fixtures/rdna2/float-exact.bin");
        assert_eq!(
            execute(fixture, &[0, u32::MAX]),
            Ok(vec![0x410c_0000, 0x4120_0000])
        );
        let mut broken = *fixture;
        broken[36..40].copy_from_slice(&1u32.to_le_bytes());
        assert_eq!(
            execute(&broken, &[0]).unwrap_err().detail,
            "float operation outside positive, normal, exact binary32 domain"
        );
    }

    #[test]
    fn rejects_truncated_programs_and_trailing_bytes() {
        for length in 0..FIXTURE.len() {
            assert!(
                execute(&FIXTURE[..length], &[0]).is_err(),
                "length {length}"
            );
        }
        let mut trailing = FIXTURE.to_vec();
        trailing.push(0);
        assert_eq!(execute(&trailing, &[0]).unwrap_err().offset, FIXTURE.len());
    }

    #[test]
    fn rejects_access_before_wait() {
        let mut no_read_wait = FIXTURE.to_vec();
        no_read_wait.drain(8..12);
        assert_eq!(
            execute(&no_read_wait, &[1]).unwrap_err().detail,
            "VGPR read before load wait"
        );
        let mut no_write_wait = FIXTURE.to_vec();
        no_write_wait.drain(36..40);
        assert_eq!(
            execute(&no_write_wait, &[1]).unwrap_err().detail,
            "outstanding transfer at termination"
        );
    }

    #[test]
    fn decodes_high_register_indices() {
        // V_MOV_B32 v255, v254 and V_ADD_NC_U32 v255, v128, v255.
        let code = [
            0xfe, 0x03, 0xfe, 0x7f, 0xff, 0, 0x25, 0xd5, 0x80, 0xff, 0x03, 0x02, 0, 0, 0x81, 0xbf,
        ];
        assert_eq!(
            decode(&code),
            Ok(vec![
                DecodedInstruction {
                    offset: 0,
                    byte_len: 4,
                    op: Op::Move(255, Source::Register(254))
                },
                DecodedInstruction {
                    offset: 4,
                    byte_len: 8,
                    op: Op::Add(255, 128, 255)
                },
                DecodedInstruction {
                    offset: 12,
                    byte_len: 4,
                    op: Op::End
                },
            ])
        );
    }
}
