// SPDX-License-Identifier: GPL-3.0-or-later

//! Verified compilation for Xenora's synthetic compute ABI.

use std::fmt::{self, Write};

pub use xenora_decoder::Architecture;
use xenora_decoder::{DecodeError, DecodedInstruction, Instruction, ScalarSource, VectorSource};

/// Entry point of all kernels produced by this compiler.
pub const ENTRY_POINT: &str = "xenora_compute";
const MAX_CODE_BYTES: usize = 1024;

#[derive(Debug)]
pub struct CompiledShader {
    pub disassembly: String,
    pub ir: String,
    pub msl: String,
}

#[derive(Debug)]
pub enum CompileError {
    InvalidLength { bytes: usize },
    Decode(DecodeError),
    Verify { offset: usize, reason: &'static str },
}

impl fmt::Display for CompileError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLength { bytes } => write!(
                formatter,
                "fixture length must be a nonzero multiple of four up to {MAX_CODE_BYTES} bytes, got {bytes}"
            ),
            Self::Decode(error) => error.fmt(formatter),
            Self::Verify { offset, reason } => {
                write!(
                    formatter,
                    "shader verification at byte 0x{offset:x}: {reason}"
                )
            }
        }
    }
}

impl std::error::Error for CompileError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Decode(error) => Some(error),
            _ => None,
        }
    }
}

#[derive(Clone, Copy)]
struct Value(usize);

impl fmt::Display for Value {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "t{}", self.0)
    }
}

#[derive(Clone, Copy)]
enum Op {
    Load,
    Constant(u32),
    Binary(Binary, Value, Value),
    Store(Value),
    ScalarBranch { target: usize, taken: bool },
}

#[derive(Clone, Copy)]
enum Binary {
    Add,
    And,
    Xor,
    FloatAdd,
    FloatMul,
}

impl Binary {
    fn operator(self) -> (&'static str, char) {
        match self {
            Self::Add => ("add.u32", '+'),
            Self::And => ("and.b32", '&'),
            Self::Xor => ("xor.b32", '^'),
            Self::FloatAdd => ("add.f32", '+'),
            Self::FloatMul => ("mul.f32", '*'),
        }
    }
}

// Each bit is either known zero, known one or unconstrained.
#[derive(Clone, Copy)]
struct KnownBits {
    zero: u32,
    one: u32,
    values: Option<[u32; 2]>,
}

impl KnownBits {
    const UNKNOWN: Self = Self {
        zero: 0,
        one: 0,
        values: None,
    };

    fn constant(value: u32) -> Self {
        Self {
            zero: !value,
            one: value,
            values: Some([value; 2]),
        }
    }

    fn and(self, other: Self) -> Self {
        Self {
            zero: self.zero | other.zero,
            one: self.one & other.one,
            values: None,
        }
    }

    fn xor(self, other: Self) -> Self {
        Self {
            zero: (self.zero & other.zero) | (self.one & other.one),
            one: (self.zero & other.one) | (self.one & other.zero),
            values: None,
        }
    }

    fn positive_normal_candidates(self) -> Option<[f32; 2]> {
        let values = match self.values {
            Some(values) => values,
            None => {
                let unknown = !(self.zero | self.one);
                if unknown.count_ones() > 1 {
                    return None;
                }
                [self.one, self.one | unknown]
            }
        };
        let candidates = values.map(f32::from_bits);
        candidates
            .iter()
            .all(|n| n.is_normal() && n.is_sign_positive())
            .then_some(candidates)
    }
}

fn exact_float_result(operator: Binary, left: KnownBits, right: KnownBits) -> Option<KnownBits> {
    let left = left.positive_normal_candidates()?;
    let right = right.positive_normal_candidates()?;
    let mut zero = u32::MAX;
    let mut one = u32::MAX;
    let mut results = Vec::with_capacity(4);
    for a in left.map(f64::from) {
        for b in right.map(f64::from) {
            let exact = match operator {
                Binary::FloatAdd => a + b,
                Binary::FloatMul => a * b,
                _ => return None,
            };
            let value = exact as f32;
            if !value.is_normal() || f64::from(value) != exact {
                return None;
            }
            // Binary64 can round away the smaller operand of a widely separated
            // binary32 sum. Require that subtraction recovers the smaller input.
            if matches!(operator, Binary::FloatAdd) && exact - a.max(b) != a.min(b) {
                return None;
            }
            let bits = value.to_bits();
            zero &= !bits;
            one &= bits;
            if !results.contains(&bits) {
                results.push(bits);
            }
        }
    }
    let values = match results.as_slice() {
        [only] => Some([*only; 2]),
        [first, second] => Some([*first, *second]),
        _ => None,
    };
    Some(KnownBits { zero, one, values })
}

/// Compiles explicitly tagged raw machine code to a synthetic Metal compute kernel.
///
/// Bind equal-length disjoint input and output word buffers and dispatch one
/// invocation per word in a one-dimensional grid.
pub fn compile(architecture: Architecture, bytes: &[u8]) -> Result<CompiledShader, CompileError> {
    if bytes.is_empty() || bytes.len() > MAX_CODE_BYTES || !bytes.len().is_multiple_of(4) {
        return Err(CompileError::InvalidLength { bytes: bytes.len() });
    }
    let decoded = xenora_decoder::decode(architecture, bytes).map_err(CompileError::Decode)?;
    let operations = lift(&decoded, bytes.len())?;

    let disassembly = decoded.iter().map(|line| format!("{line}\n")).collect();
    let mut ir = String::new();
    let mut msl = format!(
        "#include <metal_stdlib>\nusing namespace metal;\nkernel void {ENTRY_POINT}(\n    device const uint* input [[buffer(0)]],\n    device uint* output [[buffer(1)]],\n    uint index [[thread_position_in_grid]]) {{\n"
    );
    if operations.iter().any(|(_, operation)| {
        matches!(
            operation,
            Op::Binary(Binary::FloatAdd | Binary::FloatMul, _, _)
        )
    }) {
        msl.push_str("    #pragma clang fp contract(off)\n    #pragma clang fp reassociate(off)\n");
    }
    for (index, (offset, operation)) in operations.into_iter().enumerate() {
        let dest = Value(index);
        let (step, statement) = match operation {
            Op::Load => (
                format!("{dest} = load.u32 input[index]"),
                format!("uint {dest} = input[index];"),
            ),
            Op::Constant(value) => (
                format!("{dest} = const.u32 0x{value:08x}"),
                format!("uint {dest} = 0x{value:08x}u;"),
            ),
            Op::Binary(operator, left, right) => {
                let (mnemonic, symbol) = operator.operator();
                let expression = match operator {
                    Binary::FloatAdd | Binary::FloatMul => format!(
                        "as_type<uint>(as_type<float>({left}) {symbol} as_type<float>({right}))"
                    ),
                    _ => format!("{left} {symbol} {right}"),
                };
                (
                    format!("{dest} = {mnemonic} {left}, {right}"),
                    format!("uint {dest} = {expression};"),
                )
            }
            Op::ScalarBranch { target, taken } => {
                let _ = writeln!(
                    ir,
                    "{offset:04x}: branch.uniform 0x{target:04x} taken={taken}"
                );
                continue;
            }
            Op::Store(value) => (
                format!("store.u32 output[index], {value}"),
                format!("output[index] = {value};"),
            ),
        };
        // Writing to a String is infallible.
        let _ = writeln!(ir, "{offset:04x}: {step}");
        let _ = writeln!(msl, "    {statement}");
    }
    msl.push_str("}\n");
    Ok(CompiledShader {
        disassembly,
        ir,
        msl,
    })
}

fn lift(decoded: &[DecodedInstruction], code_len: usize) -> Result<Vec<(usize, Op)>, CompileError> {
    let mut operations = Vec::new();
    let mut registers = [None; 256];
    let mut facts = Vec::<KnownBits>::new();
    let mut pending_load = None;
    let mut stored = false;
    let mut store_pending = false;
    let mut scalars = [None; 128];
    let mut scc = None;
    let mut skip_before = 0;

    for instruction in decoded {
        let target = match instruction.instruction {
            Instruction::Branch { target } | Instruction::BranchSccZero { target } => target,
            _ => continue,
        };
        if target <= instruction.offset
            || decoded
                .binary_search_by_key(&target, |instruction| instruction.offset)
                .is_err()
        {
            return Err(CompileError::Verify {
                offset: instruction.offset,
                reason: "branch target must be a forward instruction boundary",
            });
        }
    }

    for (index, instruction) in decoded.iter().enumerate() {
        let offset = instruction.offset;
        let error = |reason| CompileError::Verify { offset, reason };
        let read = |register: u8| {
            registers[usize::from(register)].ok_or_else(|| {
                error("source register is uninitialized, pending a load wait, or reserved for indexing")
            })
        };
        if offset < skip_before {
            continue;
        }
        let read_scalar = |source| match source {
            ScalarSource::Immediate(value) => Ok(value),
            ScalarSource::Register(register) => scalars
                .get(usize::from(register))
                .copied()
                .flatten()
                .ok_or_else(|| error("scalar source has no known value")),
        };
        let next = Value(operations.len());
        let (dest, operation) = match instruction.instruction {
            Instruction::BufferLoadDword {
                data,
                index: 0,
                resource: 0,
            } => {
                let slot = usize::from(data);
                if data == 0 || pending_load.is_some() {
                    return Err(error(
                        "load writes the index register or another load is outstanding",
                    ));
                }
                registers[slot] = None;
                pending_load = Some((slot, next));
                (None, Op::Load)
            }
            Instruction::BufferStoreDword {
                data,
                index: 0,
                resource: 4,
            } => {
                if stored {
                    return Err(error("only one output store is supported"));
                }
                stored = true;
                store_pending = true;
                (None, Op::Store(read(data)?))
            }
            Instruction::BufferLoadDword { .. } | Instruction::BufferStoreDword { .. } => {
                return Err(error("unsupported resource or buffer index"));
            }
            Instruction::WaitLoad => {
                if let Some((register, value)) = pending_load.take() {
                    registers[register] = Some(value);
                }
                continue;
            }
            Instruction::WaitStore => {
                store_pending = false;
                continue;
            }
            Instruction::MoveB32 {
                dst,
                src: VectorSource::Register(src),
            } => {
                let value = read(src)?;
                let slot = usize::from(dst);
                if dst == 0 || pending_load.is_some_and(|(register, _)| register == slot) {
                    return Err(error(
                        "move writes the index register or replaces a pending load",
                    ));
                }
                registers[slot] = Some(value);
                continue;
            }
            Instruction::MoveB32 {
                dst,
                src: VectorSource::Immediate(value),
            } => (Some(dst), Op::Constant(value)),
            Instruction::AddNoCarryU32 { dst, src0, src1 } => {
                (Some(dst), Op::Binary(Binary::Add, read(src0)?, read(src1)?))
            }
            Instruction::VectorAndB32 { dst, src0, src1 } => {
                (Some(dst), Op::Binary(Binary::And, read(src0)?, read(src1)?))
            }
            Instruction::VectorXorB32 { dst, src0, src1 } => {
                (Some(dst), Op::Binary(Binary::Xor, read(src0)?, read(src1)?))
            }
            Instruction::ScalarMoveB32 { dst, src } => {
                let value = read_scalar(src)?;
                if dst < 8 {
                    return Err(error("scalar move would modify a resource descriptor"));
                }
                let slot = scalars
                    .get_mut(usize::from(dst))
                    .ok_or_else(|| error("scalar destination exceeds the fixture register file"))?;
                *slot = Some(value);
                continue;
            }
            Instruction::ScalarCompareEqU32 { src0, src1 } => {
                scc = Some(read_scalar(src0)? == read_scalar(src1)?);
                continue;
            }
            Instruction::BranchSccZero { target } => {
                let taken =
                    !scc.ok_or_else(|| error("conditional branch reads uninitialized SCC"))?;
                if taken {
                    skip_before = target;
                }
                (None, Op::ScalarBranch { target, taken })
            }
            Instruction::Branch { target } => {
                skip_before = target;
                (
                    None,
                    Op::ScalarBranch {
                        target,
                        taken: true,
                    },
                )
            }
            Instruction::VectorAddF32 { dst, src0, src1 } => (
                Some(dst),
                Op::Binary(Binary::FloatAdd, read(src0)?, read(src1)?),
            ),
            Instruction::VectorMulF32 { dst, src0, src1 } => (
                Some(dst),
                Op::Binary(Binary::FloatMul, read(src0)?, read(src1)?),
            ),
            Instruction::EndProgram => {
                if !stored || store_pending || pending_load.is_some() {
                    return Err(error(
                        "termination requires one completed store and no pending loads",
                    ));
                }
                if let Some(next) = decoded.get(index + 1) {
                    return Err(CompileError::Verify {
                        offset: next.offset,
                        reason: "instruction follows program termination",
                    });
                }
                return Ok(operations);
            }
        };
        if let Some(dest) = dest {
            let slot = usize::from(dest);
            if dest == 0 || pending_load.is_some_and(|(register, _)| register == slot) {
                return Err(error(
                    "instruction writes the index register or replaces a pending load",
                ));
            }
            registers[slot] = Some(next);
        }
        let value_bits = match operation {
            Op::Constant(bits) => KnownBits::constant(bits),
            Op::Binary(Binary::And, left, right) => facts[left.0].and(facts[right.0]),
            Op::Binary(Binary::Xor, left, right) => facts[left.0].xor(facts[right.0]),
            Op::Binary(operator @ (Binary::FloatAdd | Binary::FloatMul), left, right) => {
                exact_float_result(operator, facts[left.0], facts[right.0])
                    .ok_or_else(|| error("FP32 operation is not proven exact for positive normal operands and results"))?
            }
            _ => KnownBits::UNKNOWN,
        };
        facts.push(value_bits);
        operations.push((offset, operation));
    }
    Err(CompileError::Verify {
        offset: code_len,
        reason: "missing program termination",
    })
}
