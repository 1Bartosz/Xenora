// SPDX-License-Identifier: GPL-3.0-or-later

//! Independent CPU execution of the synthetic gfx1030 fixture subset.

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
    Load { data: u8, index: u8, resource: u8 },
    Store { data: u8, index: u8, resource: u8 },
    WaitRead,
    WaitWrite,
    Move(u8, Source),
    Add(u8, u8, u8),
    ScalarMove(u8, Source),
    CompareEqual(Source, Source),
    Branch { target: usize, if_scc_zero: bool },
    Vector(VectorAlu, u8, u8, u8),
    End,
}

/// VOP2 operations decoded by the reference interpreter.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VectorAlu {
    And,
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
    input
        .iter()
        .map(|&word| execute_lane(&program, word))
        .collect()
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
            [source, 3, destination @ 0x80..=0xe9, 0xbe, tail @ ..] => {
                let (source, length) = decode_scalar(*source, tail, offset)?;
                (Op::ScalarMove(destination & 0x7f, source), length)
            }
            [left, right, 6, 0xbf, tail @ ..] if (*left, *right) != (255, 255) => {
                let (left, left_length) = decode_scalar(*left, tail, offset)?;
                let (right, right_length) = decode_scalar(*right, tail, offset)?;
                (Op::CompareEqual(left, right), left_length.max(right_length))
            }
            [lo, hi, opcode @ (0x82 | 0x84), 0xbf, ..] => {
                let words = i16::from_le_bytes([*lo, *hi]);
                let words = usize::try_from(words).map_err(|_| error(offset, "backward branch"))?;
                (
                    Op::Branch {
                        target: offset + 4 + words * 4,
                        if_scc_zero: *opcode == 0x84,
                    },
                    4,
                )
            }
            [first, second, third, opcode, ..]
                if matches!(opcode & 0xfe, 0x06 | 0x10 | 0x36 | 0x3a) && second & 1 != 0 =>
            {
                let kind = match opcode & 0xfe {
                    0x06 => VectorAlu::FloatAdd,
                    0x10 => VectorAlu::FloatMultiply,
                    0x36 => VectorAlu::And,
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
            [lo, packed, upper, tag @ 0x7e..=0x7f, tail @ ..]
                if packed & 0xfe == 2 && upper & 1 == 0 =>
            {
                let dst = (upper >> 1) | ((tag & 1) << 7);
                let selector = u16::from(*lo) | (u16::from(packed & 1) << 8);
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
        VectorAlu::And => return Ok(a & b),
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
        VectorAlu::And | VectorAlu::Xor => unreachable!(),
    };
    let result = exact as f32;
    if !result.is_normal() || f64::from(result) != exact {
        return Err(error());
    }
    Ok(result.to_bits())
}

fn execute_lane(program: &[DecodedInstruction], input: u32) -> Result<u32, ReferenceError> {
    let mut registers = [None; 256];
    let mut scalars = [None; 106];
    let mut scc = None;
    let mut target = 0;
    let mut pending_read = None;
    let mut pending_write = None;
    let mut output = None;
    for &DecodedInstruction { offset, op, .. } in program {
        if offset < target {
            continue;
        }
        let read_scalar = |source| match source {
            Source::Register(reg) => scalars[usize::from(reg)]
                .ok_or_else(|| error(offset, "read from uninitialized SGPR")),
            Source::Immediate(value) => Ok(value),
        };
        let read = |register: u8| {
            if register == 0 {
                return Err(error(offset, "v0 is reserved for the resource index"));
            }
            if pending_read == Some(register) {
                return Err(error(offset, "VGPR read before load wait"));
            }
            registers[usize::from(register)]
                .ok_or_else(|| error(offset, "read from uninitialized VGPR"))
        };
        let set = match op {
            Op::Load { data: dst, .. } => {
                if dst == 0 || pending_read.is_some() {
                    return Err(error(
                        offset,
                        "invalid load destination or overlapping load",
                    ));
                }
                pending_read = Some(dst);
                None
            }
            Op::Store { data: src, .. } => {
                let value = read(src)?;
                if pending_write.is_some() || output.is_some() {
                    return Err(error(offset, "multiple writes to the output"));
                }
                pending_write = Some(value);
                None
            }
            Op::WaitRead => pending_read.take().map(|dst| (dst, input)),
            Op::WaitWrite => {
                if let Some(value) = pending_write.take() {
                    output = Some(value);
                }
                None
            }
            Op::Move(dst, Source::Register(src)) => Some((dst, read(src)?)),
            Op::Move(dst, Source::Immediate(value)) => Some((dst, value)),
            Op::Add(dst, lhs, rhs) => Some((dst, read(lhs)?.wrapping_add(read(rhs)?))),
            Op::ScalarMove(dst, src) => {
                if dst < 8 {
                    return Err(error(offset, "write to a reserved resource SGPR"));
                }
                scalars[usize::from(dst)] = Some(read_scalar(src)?);
                None
            }
            Op::CompareEqual(lhs, rhs) => {
                scc = Some(read_scalar(lhs)? == read_scalar(rhs)?);
                None
            }
            Op::Branch {
                target: destination,
                if_scc_zero,
            } => {
                if !if_scc_zero
                    || !scc.ok_or_else(|| error(offset, "branch with uninitialized SCC"))?
                {
                    target = destination;
                }
                None
            }
            Op::Vector(operator, dst, lhs, rhs) => {
                Some((dst, vector_alu(operator, read(lhs)?, read(rhs)?, offset)?))
            }
            Op::End => {
                if pending_read.is_some() || pending_write.is_some() {
                    return Err(error(offset, "outstanding transfer at termination"));
                }
                return output.ok_or_else(|| error(offset, "lane did not write its output"));
            }
        };
        if let Some((dst, value)) = set {
            if dst == 0 || pending_read == Some(dst) {
                return Err(error(offset, "write to immutable or pending VGPR"));
            }
            registers[usize::from(dst)] = Some(value);
        }
    }
    Err(error(0, "missing decoded termination"))
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
