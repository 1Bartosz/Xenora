// SPDX-License-Identifier: GPL-3.0-or-later

//! Expected results transcribed from the AMD "RDNA 2" Instruction Set
//! Architecture Reference Guide (30 November 2020). Section numbers refer to
//! that guide. Nothing here comes from Xenora's own decoder, ISA tables,
//! compiler or semantic registry.

/// The SOP2 forms under test with their opcode numbers (guide 12.1).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Sop2 {
    AddU32 = 0,
    SubU32 = 1,
    AddI32 = 2,
    SubI32 = 3,
    AddcU32 = 4,
    SubbU32 = 5,
    AndB32 = 14,
    OrB32 = 16,
    XorB32 = 18,
}

pub const SOP2_FORMS: [(Sop2, &str); 9] = [
    (Sop2::AddU32, "s_add_u32"),
    (Sop2::SubU32, "s_sub_u32"),
    (Sop2::AddI32, "s_add_i32"),
    (Sop2::SubI32, "s_sub_i32"),
    (Sop2::AddcU32, "s_addc_u32"),
    (Sop2::SubbU32, "s_subb_u32"),
    (Sop2::AndB32, "s_and_b32"),
    (Sop2::OrB32, "s_or_b32"),
    (Sop2::XorB32, "s_xor_b32"),
];

fn bit31(value: u32) -> bool {
    value >> 31 == 1
}

/// D and SCC as guide 12.1 writes them; `scc` is the SCC value before the
/// instruction. The `>= 0x100000000ULL` tests are evaluated in 64 bits, as the
/// ULL constant says.
pub fn sop2(op: Sop2, s0: u32, s1: u32, scc: bool) -> (u32, bool) {
    let (a, b, c) = (u64::from(s0), u64::from(s1), u64::from(scc));
    match op {
        // D.u32 = S0.u32 + S1.u32; SCC = S0.u32 + S1.u32 >= 0x100000000ULL.
        Sop2::AddU32 => (s0.wrapping_add(s1), a + b >= 0x1_0000_0000),
        // D.u = S0.u - S1.u; SCC = (S1.u > S0.u).
        Sop2::SubU32 => (s0.wrapping_sub(s1), s1 > s0),
        // D.i = S0.i + S1.i; SCC = (S0.u[31] == S1.u[31] && S0.u[31] != D.u[31]).
        Sop2::AddI32 => {
            let d = s0.wrapping_add(s1);
            (d, bit31(s0) == bit31(s1) && bit31(s0) != bit31(d))
        }
        // D.i = S0.i - S1.i; SCC = (S0.u[31] != S1.u[31] && S0.u[31] != D.u[31]).
        Sop2::SubI32 => {
            let d = s0.wrapping_sub(s1);
            (d, bit31(s0) != bit31(s1) && bit31(s0) != bit31(d))
        }
        // D.u32 = S0.u32 + S1.u32 + SCC; SCC = S0.u32 + S1.u32 + SCC >= 0x100000000ULL.
        Sop2::AddcU32 => (
            s0.wrapping_add(s1).wrapping_add(u32::from(scc)),
            a + b + c >= 0x1_0000_0000,
        ),
        // D.u = S0.u - S1.u - SCC; SCC = (S1.u + SCC > S0.u), "unsigned overflow".
        // The guide does not say whether S1.u + SCC wraps at 32 bits. It is read
        // here at full width: only then does SCC report the borrow out of the
        // subtraction, which the comment names. This decides only S1 = 0xffffffff
        // with SCC set; `sop2::cases` keeps those in their own family.
        Sop2::SubbU32 => (s0.wrapping_sub(s1).wrapping_sub(u32::from(scc)), b + c > a),
        // D = S0 & S1, S0 | S1, S0 ^ S1; SCC = (D != 0).
        Sop2::AndB32 => (s0 & s1, s0 & s1 != 0),
        Sop2::OrB32 => (s0 | s1, s0 | s1 != 0),
        Sop2::XorB32 => (s0 ^ s1, s0 ^ s1 != 0),
    }
}

/// Integer inline constants (guide 5.2, scalar source operand table): codes
/// 128-192 are 0..=64 and codes 193-208 are -1..=-16.
pub fn inline_integer(code: u32) -> Option<u32> {
    match code {
        128..=192 => Some(code - 128),
        193..=208 => Some((192_i32 - code as i32) as u32),
        _ => None,
    }
}

/// Literal constant code: one 32-bit word follows the instruction (guide 5.2).
pub const LITERAL: u32 = 255;

/// The VOPC integer compares (guide 12.9): V_CMP_{LT,EQ,LE,GT,NE,GE}_I32 are
/// opcodes 129-134 and the _U32 forms 193-198.
#[derive(Clone, Copy, Debug)]
pub struct Compare {
    pub opcode: u32,
    pub name: &'static str,
}

pub const COMPARES: [Compare; 12] = [
    Compare {
        opcode: 129,
        name: "v_cmp_lt_i32",
    },
    Compare {
        opcode: 130,
        name: "v_cmp_eq_i32",
    },
    Compare {
        opcode: 131,
        name: "v_cmp_le_i32",
    },
    Compare {
        opcode: 132,
        name: "v_cmp_gt_i32",
    },
    Compare {
        opcode: 133,
        name: "v_cmp_ne_i32",
    },
    Compare {
        opcode: 134,
        name: "v_cmp_ge_i32",
    },
    Compare {
        opcode: 193,
        name: "v_cmp_lt_u32",
    },
    Compare {
        opcode: 194,
        name: "v_cmp_eq_u32",
    },
    Compare {
        opcode: 195,
        name: "v_cmp_le_u32",
    },
    Compare {
        opcode: 196,
        name: "v_cmp_gt_u32",
    },
    Compare {
        opcode: 197,
        name: "v_cmp_ne_u32",
    },
    Compare {
        opcode: 198,
        name: "v_cmp_ge_u32",
    },
];

/// "D[threadId] = (S0 < S1)" and so on (guide 12.9), with I32 operands read as
/// two's complement and U32 operands as unsigned (chapter 12 suffix rules).
pub fn compare(form: Compare, s0: u32, s1: u32) -> bool {
    let signed = form.name.ends_with("_i32");
    let order = if signed {
        (s0 as i32).cmp(&(s1 as i32))
    } else {
        s0.cmp(&s1)
    };
    match &form.name[6..8] {
        "lt" => order.is_lt(),
        "eq" => order.is_eq(),
        "le" => order.is_le(),
        "gt" => order.is_gt(),
        "ne" => order.is_ne(),
        "ge" => order.is_ge(),
        other => unreachable!("{other}"),
    }
}

/// VCC for one Wave32 wave (guide 3.9): "VCC[n] = EXEC[n] & (test passed for
/// thread[n])", and "VCC is fully written", so inactive lanes read 0.
pub fn vcc(exec: u32, passed: impl Fn(usize) -> bool) -> u32 {
    (0..32)
        .filter(|&lane| exec >> lane & 1 == 1 && passed(lane))
        .fold(0, |mask, lane| mask | 1 << lane)
}
