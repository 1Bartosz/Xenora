// SPDX-License-Identifier: GPL-3.0-or-later
const CF: u64 = 1;
const PF: u64 = 1 << 2;
const AF: u64 = 1 << 4;
const ZF: u64 = 1 << 6;
const SF: u64 = 1 << 7;
const OF: u64 = 1 << 11;
const ARITHMETIC: u64 = CF | PF | AF | ZF | SF | OF;
pub(crate) const MAX_CODE: usize = 64;
pub(crate) const MAX_STEPS: usize = 32;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct State {
    pub(crate) gpr: [u64; 16],
    pub(crate) rip: u64,
    pub(crate) rflags: u64,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum FaultKind {
    CodeRange,
    Unsupported,
    Truncated,
    BranchTarget,
    Budget,
    OutputRange,
    UnknownService,
    #[cfg(any(test, feature = "metal"))]
    InvalidCommand,
    #[cfg(feature = "metal")]
    DispatchFailed,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Fault {
    pub(crate) kind: FaultKind,
    pub(crate) rip: u64,
}

pub(crate) type Service<'a> = dyn FnMut(&State) -> Result<u64, FaultKind> + 'a;

enum Instruction {
    Mov(u32),
    AddOne,
    SubOne,
    CmpImm8(u8),
    Je(i8),
    Jne(i8),
    Store,
    Trap,
}

fn decode(bytes: &[u8], services: bool) -> Result<(Instruction, usize), FaultKind> {
    use Instruction::*;
    match bytes {
        [0x0f, 0x0b, ..] if services => Ok((Trap, 2)),
        [0xb8, a, b, c, d, ..] => Ok((Mov(u32::from_le_bytes([*a, *b, *c, *d])), 5)),
        [0x83, 0xc0, 1, ..] => Ok((AddOne, 3)),
        [0x83, 0xe8, 1, ..] => Ok((SubOne, 3)),
        [0x83, 0xf8, imm, ..] => Ok((CmpImm8(*imm), 3)),
        [0x74, disp, ..] => Ok((Je(i8::from_le_bytes([*disp])), 2)),
        [0x75, disp, ..] => Ok((Jne(i8::from_le_bytes([*disp])), 2)),
        [0x48, 0x89, 0x07, ..] => Ok((Store, 3)),
        [0xb8, ..]
        | [0x83]
        | [0x83, 0xc0 | 0xe8 | 0xf8]
        | [0x74]
        | [0x75]
        | [0x48]
        | [0x48, 0x89] => Err(FaultKind::Truncated),
        _ => Err(FaultKind::Unsupported),
    }
}

fn branch_target(
    code: &[u8],
    next: usize,
    displacement: i8,
    services: bool,
) -> Result<usize, FaultKind> {
    let target = next
        .checked_add_signed(isize::from(displacement))
        .filter(|&target| target < code.len())
        .ok_or(FaultKind::BranchTarget)?;
    // Validate that `target` falls on an instruction boundary by scanning from
    // offset 0.  This rejects branches into the middle of immediates.
    let mut boundary = 0;
    while boundary < target {
        let (_, len) = decode(&code[boundary..], services).map_err(|_| FaultKind::BranchTarget)?;
        boundary += len;
    }
    if boundary != target || decode(&code[target..], services).is_err() {
        return Err(FaultKind::BranchTarget);
    }
    Ok(target)
}

fn result_flags(value: u32) -> u64 {
    let parity = value.to_le_bytes()[0].count_ones().is_multiple_of(2);
    (u64::from(parity) * PF) | (u64::from(value == 0) * ZF) | (u64::from(value >> 31) * SF)
}

/// Compute RFLAGS arithmetic bits for a subtraction `minuend - subtrahend = result`.
/// Identical to x86-64 SUB and CMP semantics for 32-bit operands.
fn sub_flags(minuend: u32, subtrahend: u32) -> u64 {
    let result = minuend.wrapping_sub(subtrahend);
    // CF: unsigned borrow.
    let carry = minuend < subtrahend;
    // AF: borrow out of bit 3 (low nibble).
    let af = (minuend & 0xf) < (subtrahend & 0xf);
    // OF: signed overflow on subtraction.
    let overflow = ((minuend ^ subtrahend) & (minuend ^ result)) >> 31 != 0;
    result_flags(result)
        | (u64::from(carry) * CF)
        | (u64::from(af) * AF)
        | (u64::from(overflow) * OF)
}

fn step(
    code: &[u8],
    base: u64,
    offset: usize,
    state: &mut State,
    output_base: u64,
    output: &mut [u8; 8],
    service: Option<&mut Service<'_>>,
) -> Result<(), FaultKind> {
    let services = service.is_some();
    let (instruction, len) = decode(&code[offset..], services)?;
    let mut next = offset + len;
    // The low four bytes are EAX on every host.
    let [a, b, c, d, ..] = state.gpr[0].to_le_bytes();
    let eax = u32::from_le_bytes([a, b, c, d]);
    match instruction {
        Instruction::Mov(value) => state.gpr[0] = u64::from(value),
        Instruction::AddOne => {
            let (result, carry) = eax.overflowing_add(1);
            let flags = result_flags(result)
                | (u64::from(carry) * CF)
                | (u64::from(eax & 0xf == 0xf) * AF)
                | (u64::from(eax == 0x7fff_ffff) * OF);
            state.gpr[0] = u64::from(result);
            state.rflags = (state.rflags & !ARITHMETIC) | flags;
        }
        Instruction::SubOne => {
            let flags = sub_flags(eax, 1);
            state.gpr[0] = u64::from(eax.wrapping_sub(1));
            state.rflags = (state.rflags & !ARITHMETIC) | flags;
        }
        Instruction::CmpImm8(imm) => {
            // x86-64 sign-extends the byte immediate to 32 bits before subtracting.
            let subtrahend = imm as i8 as i32 as u32;
            let flags = sub_flags(eax, subtrahend);
            state.rflags = (state.rflags & !ARITHMETIC) | flags;
        }
        Instruction::Je(displacement) => {
            let target = branch_target(code, next, displacement, services)?;
            if state.rflags & ZF != 0 {
                next = target;
            }
        }
        Instruction::Jne(displacement) => {
            let target = branch_target(code, next, displacement, services)?;
            if state.rflags & ZF == 0 {
                next = target;
            }
        }
        Instruction::Trap => {
            state.gpr[0] = service.ok_or(FaultKind::UnknownService)?(state)?;
        }
        Instruction::Store => {
            let address = state.gpr[7];
            let start = address
                .checked_sub(output_base)
                .ok_or(FaultKind::OutputRange)?;
            let end = address.checked_add(8).ok_or(FaultKind::OutputRange)?;
            let limit = output_base.checked_add(8).ok_or(FaultKind::OutputRange)?;
            if end > limit {
                return Err(FaultKind::OutputRange);
            }
            let start = usize::try_from(start).map_err(|_| FaultKind::OutputRange)?;
            output[start..start + 8].copy_from_slice(&state.gpr[0].to_le_bytes());
        }
    }
    // The complete code range was checked before execution.
    state.rip = base + next as u64;
    Ok(())
}

pub(crate) fn run(
    code: &[u8],
    state: &mut State,
    output_base: u64,
    output: &mut [u8; 8],
    budget: usize,
    retired: impl FnMut(&State),
) -> Result<(), Fault> {
    execute(code, state, output_base, output, budget, None, retired)
}

pub(crate) fn execute(
    code: &[u8],
    state: &mut State,
    output_base: u64,
    output: &mut [u8; 8],
    budget: usize,
    mut service: Option<&mut Service<'_>>,
    mut retired: impl FnMut(&State),
) -> Result<(), Fault> {
    let base = state.rip;
    let terminal = base
        .checked_add(code.len() as u64)
        .filter(|_| code.len() <= MAX_CODE);
    let terminal = terminal.ok_or(Fault {
        kind: FaultKind::CodeRange,
        rip: base,
    })?;
    for _ in 0..budget.min(MAX_STEPS) {
        if state.rip == terminal {
            break;
        }
        let offset = (state.rip - base) as usize;
        step(
            code,
            base,
            offset,
            state,
            output_base,
            output,
            service
                .as_mut()
                .map(|handler| &mut **handler as &mut Service<'_>),
        )
        .map_err(|kind| Fault {
            kind,
            rip: state.rip,
        })?;
        retired(state);
    }
    if state.rip == terminal {
        Ok(())
    } else {
        Err(Fault {
            kind: FaultKind::Budget,
            rip: state.rip,
        })
    }
}

#[cfg(test)]
mod tests;
