// SPDX-License-Identifier: GPL-3.0-or-later

//! A second, structurally independent interpreter for the demo instruction
//! subset, used only by the semantic-review tests.
//!
//! This module deliberately imports neither `xenora-decoder`, `xenora-isa` nor
//! `xenora-ref`. It re-decodes the gfx1030 encodings from the AMD RDNA 2 ISA
//! field layout and executes them with an explicit program-counter machine, so
//! a defect in the production decoder or reference cannot cancel out here.
//!
//! Scope matches the three demo fixtures: scalar move/compare/branch, VOP1
//! move, VOP2 bitwise and float, VOP3 add-no-carry, MUBUF load/store and the
//! two waits plus `s_endpgm`. Anything else is a controlled error.

const MAX_CODE_BYTES: usize = 1024;
const MAX_INPUT_WORDS: usize = 4096;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Src {
    Reg(u8),
    Imm(u32),
}

#[derive(Clone, Copy, Debug)]
enum Insn {
    SMov(u8, Src),
    SCmpEq(Src, Src),
    BranchScc0(usize),
    Branch(usize),
    VMov(u8, Src),
    VAddNc(u8, u8, u8),
    VAnd(u8, u8, u8),
    VXor(u8, u8, u8),
    VAddF(u8, u8, u8),
    VMulF(u8, u8, u8),
    Load { data: u8, index: u8, resource: u8 },
    Store { data: u8, index: u8, resource: u8 },
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
    input.iter().map(|&word| run_lane(&program, word)).collect()
}

fn run_lane(program: &[(usize, Insn)], lane_input: u32) -> Result<u32, String> {
    let mut vgpr = [None::<u32>; 256];
    let mut sgpr = [None::<u32>; 106];
    let mut scc: Option<bool> = None;
    let mut pending_read: Option<u8> = None;
    let mut pending_write: Option<u32> = None;
    let mut written: Option<u32> = None;
    let mut pc = 0usize;

    while pc < program.len() {
        let (offset, insn) = program[pc];
        let read_v = |reg: u8| -> Result<u32, String> {
            if reg == 0 {
                return Err(format!("{offset:#x}: v0 is the resource index"));
            }
            if pending_read == Some(reg) {
                return Err(format!("{offset:#x}: read before load wait"));
            }
            vgpr[reg as usize].ok_or_else(|| format!("{offset:#x}: uninitialized v{reg}"))
        };
        let read_s = |src: Src| -> Result<u32, String> {
            match src {
                Src::Imm(value) => Ok(value),
                Src::Reg(reg) => {
                    sgpr[reg as usize].ok_or_else(|| format!("{offset:#x}: uninitialized s{reg}"))
                }
            }
        };

        match insn {
            Insn::Load {
                data,
                index,
                resource,
            } => {
                if index != 0 || resource != 0 {
                    return Err(format!("{offset:#x}: load outside synthetic ABI"));
                }
                if data == 0 || pending_read.is_some() {
                    return Err(format!("{offset:#x}: invalid or overlapping load"));
                }
                pending_read = Some(data);
            }
            Insn::Store {
                data,
                index,
                resource,
            } => {
                if index != 0 || resource != 4 {
                    return Err(format!("{offset:#x}: store outside synthetic ABI"));
                }
                let value = read_v(data)?;
                if pending_write.is_some() || written.is_some() {
                    return Err(format!("{offset:#x}: multiple output writes"));
                }
                pending_write = Some(value);
            }
            Insn::WaitRead => {
                if let Some(dst) = pending_read.take() {
                    vgpr[dst as usize] = Some(lane_input);
                }
            }
            Insn::WaitWrite => {
                if let Some(value) = pending_write.take() {
                    written = Some(value);
                }
            }
            Insn::VMov(dst, src) => {
                let value = match src {
                    Src::Imm(value) => value,
                    Src::Reg(reg) => read_v(reg)?,
                };
                if dst == 0 {
                    return Err(format!("{offset:#x}: move writes v0"));
                }
                vgpr[dst as usize] = Some(value);
            }
            Insn::VAddNc(dst, a, b) => {
                let value = read_v(a)?.wrapping_add(read_v(b)?);
                vgpr[dst as usize] = Some(value);
            }
            Insn::VAnd(dst, a, b) => {
                let value = read_v(a)? & read_v(b)?;
                vgpr[dst as usize] = Some(value);
            }
            Insn::VXor(dst, a, b) => {
                let value = read_v(a)? ^ read_v(b)?;
                vgpr[dst as usize] = Some(value);
            }
            Insn::VAddF(dst, a, b) => {
                let value = exact_float(true, read_v(a)?, read_v(b)?, offset)?;
                vgpr[dst as usize] = Some(value);
            }
            Insn::VMulF(dst, a, b) => {
                let value = exact_float(false, read_v(a)?, read_v(b)?, offset)?;
                vgpr[dst as usize] = Some(value);
            }
            Insn::SMov(dst, src) => {
                if dst < 8 {
                    return Err(format!("{offset:#x}: write to a resource SGPR"));
                }
                sgpr[dst as usize] = Some(read_s(src)?);
            }
            Insn::SCmpEq(a, b) => {
                scc = Some(read_s(a)? == read_s(b)?);
            }
            Insn::BranchScc0(target) => {
                let condition = scc.ok_or_else(|| format!("{offset:#x}: uninitialized SCC"))?;
                if !condition {
                    pc = index_of(program, target, offset)?;
                    continue;
                }
            }
            Insn::Branch(target) => {
                pc = index_of(program, target, offset)?;
                continue;
            }
            Insn::End => {
                if pending_read.is_some() || pending_write.is_some() {
                    return Err(format!("{offset:#x}: outstanding transfer at end"));
                }
                if pc + 1 != program.len() {
                    return Err(format!("{offset:#x}: code after s_endpgm"));
                }
                return written.ok_or_else(|| format!("{offset:#x}: missing output store"));
            }
        }
        pc += 1;
    }
    Err("stream ended without s_endpgm".to_string())
}

fn index_of(program: &[(usize, Insn)], target: usize, offset: usize) -> Result<usize, String> {
    program
        .iter()
        .position(|(at, _)| *at == target)
        .ok_or_else(|| format!("{offset:#x}: branch target {target:#x} is not a boundary"))
}

fn exact_float(add: bool, a_bits: u32, b_bits: u32, offset: usize) -> Result<u32, String> {
    let (a, b) = (f32::from_bits(a_bits), f32::from_bits(b_bits));
    if !a.is_normal() || !b.is_normal() || a.is_sign_negative() || b.is_sign_negative() {
        return Err(format!("{offset:#x}: float outside positive normal domain"));
    }
    let (a, b) = (f64::from(a), f64::from(b));
    let exact = if add { a + b } else { a * b };
    let result = exact as f32;
    if !result.is_normal() || f64::from(result) != exact {
        return Err(format!("{offset:#x}: float result not exact binary32"));
    }
    Ok(result.to_bits())
}

fn decode(code: &[u8]) -> Result<Vec<(usize, Insn)>, String> {
    let mut program = Vec::new();
    let mut offset = 0;
    while offset < code.len() {
        let (insn, len) = decode_one(code, offset)?;
        program.push((offset, insn));
        offset += len;
    }
    for &(at, insn) in &program {
        if let Insn::Branch(target) | Insn::BranchScc0(target) = insn
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

    // Opcode masks and values are the AMD RDNA 2 ISA encoding fields.
    if first & 0xfffc_0000 == 0xe030_0000 {
        let second = word(code, offset + 4)?;
        let (data, index, resource) = mubuf(second).ok_or_else(|| fail("bad MUBUF load form"))?;
        return Ok((
            Insn::Load {
                data,
                index,
                resource,
            },
            8,
        ));
    }
    if first & 0xfffc_0000 == 0xe070_0000 {
        let second = word(code, offset + 4)?;
        let (data, index, resource) = mubuf(second).ok_or_else(|| fail("bad MUBUF store form"))?;
        return Ok((
            Insn::Store {
                data,
                index,
                resource,
            },
            8,
        ));
    }
    if first & 0xfe01_fe00 == 0x7e00_0200 {
        let dst = (first >> 17) as u8;
        let selector = first & 0x1ff;
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
        let (src, len) = scalar_source(code, offset, first as u8)?;
        return Ok((Insn::SMov(dst, src), len));
    }
    if first & 0xffff_0000 == 0xbf06_0000 {
        let a = first as u8;
        let b = (first >> 8) as u8;
        if a == 255 && b == 255 {
            return Err(fail("two scalar literals"));
        }
        let (sa, la) = scalar_source(code, offset, a)?;
        let (sb, lb) = scalar_source(code, offset, b)?;
        return Ok((Insn::SCmpEq(sa, sb), la.max(lb)));
    }
    if first & 0xffff_0000 == 0xbf82_0000 {
        return Ok((Insn::Branch(branch_target(first, offset)?), 4));
    }
    if first & 0xffff_0000 == 0xbf84_0000 {
        return Ok((Insn::BranchScc0(branch_target(first, offset)?), 4));
    }
    for (mask, value, kind) in [
        (0xfe00_0000, 0x3600_0000u32, 0u8),
        (0xfe00_0000, 0x3a00_0000, 1),
        (0xfe00_0000, 0x0600_0000, 2),
        (0xfe00_0000, 0x1000_0000, 3),
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
                _ => Insn::VMulF(dst, a, b),
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

fn mubuf(second: u32) -> Option<(u8, u8, u8)> {
    if second & 0xffe0_0000 != 0x8000_0000 {
        return None;
    }
    let index = second as u8;
    let data = (second >> 8) as u8;
    let resource = ((second >> 16) & 0x1f) as u8 * 4;
    (resource <= 102).then_some((data, index, resource))
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
