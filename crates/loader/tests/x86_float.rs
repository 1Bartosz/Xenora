// SPDX-License-Identifier: GPL-3.0-or-later

//! SSE and SSE2 floating-point tests.
//!
//! Expected values come from `tests/x86/float_vectors.txt`, computed with exact
//! fractions by `tests/x86/float_reference.py` from the Intel SDM rules, and, for
//! round-to-nearest values, from the ARM64 host's IEEE 754 hardware. Encodings
//! come from LLVM (`tests/x86/float.txt`). Neither shares code with
//! `softfloat.rs`.

use std::collections::BTreeMap;
use xenora_loader::cpu::{CpuState, ExecutionResult, ExitReason, Interpreter, Mxcsr, RFlags};
use xenora_loader::dbt::DbtEngine;
use xenora_loader::memory::{GuestMemory, MemoryPermissions};
use xenora_loader::softfloat::{self, Arithmetic, Control, F32, F64, Format};

const ENTRY: u64 = 0x1000;
const DATA: u64 = 0x2000;
/// Address of `[rsi + 4*rdi + 16]` with RSI = DATA and RDI = 4.
const OPERAND: u64 = DATA + 32;
const FILLER: u128 = 0x1122_3344_5566_7788_99AA_BBCC_DDEE_FF00;

fn fixture() -> BTreeMap<&'static str, Vec<u8>> {
    include_str!("x86/float.txt")
        .lines()
        .map(|line| {
            let (hex, text) = line.split_once(" | ").unwrap();
            let code = hex
                .split(' ')
                .map(|b| u8::from_str_radix(b, 16).unwrap())
                .collect();
            (text, code)
        })
        .collect()
}

#[derive(Clone, Copy, Debug)]
struct Vector {
    name: &'static str,
    mxcsr: u32,
    a: u64,
    b: u64,
    result: u64,
    flags: u32,
}

fn vectors() -> Vec<Vector> {
    include_str!("x86/float_vectors.txt")
        .lines()
        .map(|line| {
            let parts: Vec<&str> = line.split(' ').collect();
            let hex = |index: usize| u64::from_str_radix(parts[index], 16).unwrap();
            Vector {
                name: parts[0],
                mxcsr: hex(1) as u32,
                a: hex(2),
                b: hex(3),
                result: hex(4),
                flags: hex(5) as u32,
            }
        })
        .collect()
}

/// Where an operation's result goes and how wide its operands are.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    /// Lane 0 of XMM3, keeping the rest.
    Scalar {
        source: u32,
        result: u32,
    },
    /// R9 from XMM9.
    ToGeneral {
        source: u32,
    },
    /// Lane 0 of XMM3 from R9.
    FromGeneral {
        source: u32,
        result: u32,
    },
    Flags {
        source: u32,
    },
}

fn width(text: &str) -> u32 {
    if text.contains("ss") { 32 } else { 64 }
}

/// Instruction text for the register form, and the kind, of a vector's name.
fn instruction(name: &str) -> (String, Kind) {
    if name == "cvtss2sd" || name == "cvtsd2ss" {
        let source = if name == "cvtss2sd" { 32 } else { 64 };
        return (
            format!("{name} xmm3, xmm9"),
            Kind::Scalar {
                source,
                result: 96 - source,
            },
        );
    }
    if let Some(rest) = name.strip_prefix("cvtsi2") {
        let (format, size) = rest.split_at(2);
        let register = if size == "d" { "r9d" } else { "r9" };
        let source = if size == "d" { 32 } else { 64 };
        let text = format!("cvtsi2{format} xmm3, {register}");
        return (
            text,
            Kind::FromGeneral {
                source,
                result: width(format),
            },
        );
    }
    if name.contains("2si") {
        let (stem, size) = name.split_at(name.len() - 1);
        let register = if size == "d" { "r9d" } else { "r9" };
        let source = if stem.contains("ss2") { 32 } else { 64 };
        return (
            format!("{stem} {register}, xmm9"),
            Kind::ToGeneral { source },
        );
    }
    if name.contains("comi") {
        let source = if name.ends_with("ss") { 32 } else { 64 };
        return (format!("{name} xmm3, xmm9"), Kind::Flags { source });
    }
    if let Some(rest) = name.strip_prefix("cmp") {
        let (format, code) = rest.split_at(2);
        let bits = if format == "ss" { 32 } else { 64 };
        return (
            format!("cmp{format} xmm3, xmm9, {code}"),
            Kind::Scalar {
                source: bits,
                result: bits,
            },
        );
    }
    let bits = if name.ends_with("ss") { 32 } else { 64 };
    (
        format!("{name} xmm3, xmm9"),
        Kind::Scalar {
            source: bits,
            result: bits,
        },
    )
}

/// Memory-source text of a register form.
fn memory_text(text: &str, kind: Kind) -> String {
    let source = match kind {
        Kind::Scalar { source, .. }
        | Kind::ToGeneral { source }
        | Kind::FromGeneral { source, .. }
        | Kind::Flags { source } => source,
    };
    let place = format!(
        "{} ptr [rsi + 4*rdi + 16]",
        if source == 32 { "dword" } else { "qword" }
    );
    let operand = match kind {
        Kind::FromGeneral { .. } => text.rsplit(", ").next().unwrap(),
        _ => "xmm9",
    };
    let (head, tail) = text.rsplit_once(operand).unwrap();
    format!("{head}{place}{tail}")
}

fn merge(old: u128, value: u64, bits: u32, index: u32) -> u128 {
    let modulus = 1u128 << bits;
    let shift = bits * index;
    let below = old & ((1u128 << shift) - 1);
    let above = old.checked_shr(shift + bits).unwrap_or(0);
    let above = above.checked_shl(shift + bits).unwrap_or(0);
    below | ((u128::from(value) % modulus) << shift) | above
}

fn load(code: &[u8], mxcsr: u32, a: u128, b: u128, r9: u64) -> (CpuState, GuestMemory) {
    let mut memory = GuestMemory::new();
    memory
        .map_region_with_data(ENTRY, code.len(), MemoryPermissions::READ_EXECUTE, code)
        .unwrap();
    memory
        .map_region(DATA, 0x100, MemoryPermissions::READ_WRITE)
        .unwrap();
    memory.write_bytes(OPERAND, &b.to_le_bytes()).unwrap();
    let mut cpu = CpuState::new(ENTRY, 0);
    cpu.gpr[6] = DATA;
    cpu.gpr[7] = 4;
    cpu.gpr[9] = r9;
    cpu.xmm = [FILLER; 16];
    cpu.xmm[3] = a;
    cpu.xmm[9] = b;
    cpu.mxcsr = Mxcsr(mxcsr);
    cpu.flags = RFlags {
        cf: true,
        zf: false,
        sf: true,
        of: true,
        pf: false,
        af: true,
    };
    (cpu, memory)
}

fn run(native: bool, cpu: &CpuState, memory: &mut GuestMemory, budget: usize) -> ExecutionResult {
    if native {
        DbtEngine::new(64 * 1024)
            .unwrap()
            .run(&mut cpu.clone(), memory, budget)
    } else {
        Interpreter::new(cpu.clone(), memory, budget).run()
    }
}

/// Initial XMM3, source operand and R9 of a vector, with unrelated upper bits.
fn inputs(vector: &Vector, kind: Kind) -> (u128, u128, u64) {
    match kind {
        Kind::Scalar { source, .. } | Kind::Flags { source } => (
            merge(FILLER, vector.a, source, 0),
            merge(!FILLER, vector.b, source, 0),
            0,
        ),
        Kind::ToGeneral { source } => (FILLER, merge(!FILLER, vector.b, source, 0), u64::MAX),
        Kind::FromGeneral { source, .. } => {
            let upper = if source == 32 {
                0xABCD_EF01_0000_0000
            } else {
                0
            };
            // The memory form reads the same value from the operand.
            (FILLER, u128::from(vector.b), vector.b | upper)
        }
    }
}

fn expect(before: &CpuState, vector: &Vector, kind: Kind, length: usize) -> CpuState {
    let mut after = before.clone();
    after.rip = ENTRY + length as u64;
    after.mxcsr = Mxcsr(before.mxcsr.0 | vector.flags);
    match kind {
        Kind::Scalar { result, .. } | Kind::FromGeneral { result, .. } => {
            after.xmm[3] = merge(before.xmm[3], vector.result, result, 0);
        }
        Kind::ToGeneral { .. } => after.gpr[9] = vector.result,
        Kind::Flags { .. } => {
            after.flags = RFlags {
                zf: vector.result & 4 != 0,
                pf: vector.result & 2 != 0,
                cf: vector.result & 1 != 0,
                ..RFlags::default()
            };
        }
    }
    after
}

#[test]
fn scalar_vectors_match_in_both_engines_from_registers_and_memory() {
    let code = fixture();
    let all = vectors();
    assert!(all.len() > 5000);
    for vector in &all {
        let (text, kind) = instruction(vector.name);
        let (a, b, r9) = inputs(vector, kind);
        for text in [text.clone(), memory_text(&text, kind)] {
            let code = code.get(text.as_str()).unwrap_or_else(|| panic!("{text}"));
            for native in [false, true] {
                let (cpu, mut memory) = load(code, vector.mxcsr, a, b, r9);
                let result = run(native, &cpu, &mut memory, 1);
                let wanted = expect(&cpu, vector, kind, code.len());
                assert_eq!(
                    result.exit_reason,
                    ExitReason::StepLimitExceeded,
                    "{text} {vector:x?}"
                );
                assert_eq!(result.final_state, wanted, "{text} {vector:x?}");
            }
        }
    }
}

#[test]
fn sticky_flags_accumulate_and_are_never_cleared() {
    let code = fixture();
    for vector in vectors().iter().step_by(7) {
        let (text, kind) = instruction(vector.name);
        let (a, b, r9) = inputs(vector, kind);
        let code = &code[text.as_str()];
        let (cpu, mut memory) = load(code, vector.mxcsr | 0x2A, a, b, r9);
        let result = run(true, &cpu, &mut memory, 1);
        assert_eq!(
            result.final_state,
            expect(&cpu, vector, kind, code.len()),
            "{text}"
        );
    }
}

#[test]
fn packed_forms_apply_the_scalar_rules_to_every_lane() {
    let code = fixture();
    // Vectors of one operation and MXCSR, taken a register at a time.
    let mut groups: BTreeMap<(&str, u32), Vec<Vector>> = BTreeMap::new();
    for vector in vectors() {
        groups
            .entry((vector.name, vector.mxcsr))
            .or_default()
            .push(vector);
    }
    let mut checked = 0;
    for ((name, mxcsr), group) in groups {
        // (packed text, source width, result width, lanes, upper half zeroed)
        let packed = match name {
            "cvtss2sd" => ("cvtps2pd xmm3, xmm9".to_string(), 32, 64, 2),
            "cvtsd2ss" => ("cvtpd2ps xmm3, xmm9".to_string(), 64, 32, 2),
            "cvtss2sid" => ("cvtps2dq xmm3, xmm9".to_string(), 32, 32, 4),
            "cvttss2sid" => ("cvttps2dq xmm3, xmm9".to_string(), 32, 32, 4),
            "cvtsd2sid" => ("cvtpd2dq xmm3, xmm9".to_string(), 64, 32, 2),
            "cvttsd2sid" => ("cvttpd2dq xmm3, xmm9".to_string(), 64, 32, 2),
            "cvtsi2ssd" => ("cvtdq2ps xmm3, xmm9".to_string(), 32, 32, 4),
            "cvtsi2sdd" => ("cvtdq2pd xmm3, xmm9".to_string(), 32, 64, 2),
            _ if name.contains("comi") || name.contains("cvt") => continue,
            _ => {
                let (text, _) = instruction(name);
                let bits = if name.contains("ss") && !name.starts_with("cmpsd") {
                    32
                } else {
                    64
                };
                let text = if bits == 32 {
                    text.replacen("ss ", "ps ", 1)
                } else {
                    text.replacen("sd ", "pd ", 1)
                };
                (text, bits, bits, 128 / bits)
            }
        };
        let (text, source, result, lanes) = packed;
        let code = code.get(text.as_str()).unwrap_or_else(|| panic!("{text}"));
        for chunk in group.chunks_exact(lanes as usize) {
            let (mut a, mut b) = (FILLER, !FILLER);
            let mut wanted = 0;
            let mut flags = 0;
            for (index, vector) in chunk.iter().enumerate() {
                a = merge(a, vector.a, source, index as u32);
                b = merge(b, vector.b, source, index as u32);
                wanted = merge(wanted, vector.result, result, index as u32);
                flags |= vector.flags;
            }
            for native in [false, true] {
                let (cpu, mut memory) = load(code, mxcsr, a, b, 0);
                let outcome = run(native, &cpu, &mut memory, 1);
                assert_eq!(outcome.exit_reason, ExitReason::StepLimitExceeded, "{text}");
                assert_eq!(outcome.final_state.xmm[3], wanted, "{text} {chunk:x?}");
                assert_eq!(
                    outcome.final_state.mxcsr.0,
                    mxcsr | flags,
                    "{text} {chunk:x?}"
                );
                checked += 1;
            }
        }
    }
    assert!(checked > 1500, "{checked}");
}

#[test]
fn unmasked_exceptions_stop_without_a_result() {
    let code = fixture();
    let mut seen = 0;
    for vector in vectors().iter().filter(|vector| vector.flags != 0) {
        let (text, kind) = instruction(vector.name);
        let (a, b, r9) = inputs(vector, kind);
        let code = &code[text.as_str()];
        for bit in 0..6 {
            if vector.flags & 1 << bit == 0 {
                continue;
            }
            // Flush-to-zero applies only while underflow is masked.
            if bit == 4 && vector.mxcsr & 1 << 15 != 0 {
                continue;
            }
            let mxcsr = vector.mxcsr & !(1 << (bit + 7));
            for native in [false, true] {
                let (cpu, mut memory) = load(code, mxcsr, a, b, r9);
                let result = run(native, &cpu, &mut memory, 1);
                assert_eq!(
                    result.exit_reason,
                    ExitReason::SimdException,
                    "{text} {vector:x?}"
                );
                assert_eq!(result.instructions_executed, 0);
                let flags = result.final_state.mxcsr.0 ^ mxcsr;
                assert!(
                    flags & 1 << bit != 0 && flags & !vector.flags == 0,
                    "{text} {vector:x?}"
                );
                let mut unchanged = cpu.clone();
                unchanged.mxcsr = result.final_state.mxcsr;
                assert_eq!(result.final_state, unchanged, "{text} {vector:x?}");
                seen |= 1 << bit;
            }
        }
    }
    assert_eq!(seen, 0x3F);
}

#[test]
fn pre_computation_exceptions_in_any_lane_come_first() {
    let code = fixture();
    let code = &code["addps xmm3, xmm9"];
    // Lane 0 is inexact (1 + 2^-30); lane 1 is infinity minus infinity.
    let one = 0x3F80_0000u128;
    let a = one | 0x7F80_0000 << 32;
    let b = 0x3080_0000 | 0xFF80_0000u128 << 32;
    for (unmasked, flags) in [
        (1 << 7, 1),           // invalid: precision is not reported
        (1 << 12, 1 | 1 << 5), // precision: both were raised
        (1 << 7 | 1 << 12, 1), // both: only the pre-computation one
    ] {
        for native in [false, true] {
            let (cpu, mut memory) = load(code, 0x1F80 & !unmasked, a, b, 0);
            let result = run(native, &cpu, &mut memory, 1);
            assert_eq!(result.exit_reason, ExitReason::SimdException);
            assert_eq!(result.final_state.mxcsr.0, 0x1F80 & !unmasked | flags);
            assert_eq!(result.final_state.xmm, cpu.xmm);
        }
    }
}

#[test]
fn unmasked_underflow_reports_an_exact_denormal_result() {
    let code = fixture();
    let code = &code["mulss xmm3, xmm9"];
    // 2^-126 * 0.5 is exactly representable, so it underflows only if unmasked.
    let (a, b) = (0x0080_0000u128, 0x3F00_0000u128);
    for native in [false, true] {
        let (cpu, mut memory) = load(code, 0x1F80, a, b, 0);
        let result = run(native, &cpu, &mut memory, 1);
        assert_eq!(result.exit_reason, ExitReason::StepLimitExceeded);
        assert_eq!(result.final_state.mxcsr.0, 0x1F80);
        assert_eq!(result.final_state.xmm[3], 0x0040_0000);
        for ftz in [0, 1 << 15] {
            let mxcsr = 0x1F80 & !(1 << 11) | ftz;
            let (cpu, mut memory) = load(code, mxcsr, a, b, 0);
            let result = run(native, &cpu, &mut memory, 1);
            assert_eq!(result.exit_reason, ExitReason::SimdException);
            assert_eq!(result.final_state.mxcsr.0, mxcsr | 1 << 4);
        }
    }
}

#[test]
fn misaligned_packed_operands_fault() {
    for (text, code) in fixture() {
        if !text.contains("xmmword ptr") {
            continue;
        }
        for native in [false, true] {
            let (mut cpu, mut memory) = load(&code, 0x1F80, 0, 0, 0);
            cpu.gpr[6] += 8;
            let result = run(native, &cpu, &mut memory, 1);
            assert_eq!(result.exit_reason, ExitReason::GeneralProtection, "{text}");
            assert_eq!(result.final_state, cpu, "{text}");
        }
    }
}

struct Random(u64);

impl Random {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// A number biased towards nearby exponents, denormals and short fractions.
    fn bits(&mut self, format: Format) -> u64 {
        let fraction = if format.bits == 32 { 23 } else { 52 };
        let mask = if format.bits == 32 {
            u64::from(u32::MAX)
        } else {
            u64::MAX
        };
        let raw = self.next() & mask;
        let sign = raw & 1 << (format.bits - 1);
        match self.next() % 6 {
            0 => raw & !(0x7FFu64 << fraction & mask) | sign,
            1 => {
                let exponent_bias = (1u64 << (format.bits - 2 - fraction)) - 1;
                let exponent = exponent_bias - 30 + self.next() % 61;
                sign | exponent << fraction | raw & ((1 << fraction) - 1)
            }
            2 => raw & !((1 << (fraction - 3)) - 1),
            _ => raw,
        }
    }
}

const NEAREST: u32 = 0x1F80;

#[test]
fn nearest_rounding_agrees_with_host_hardware() {
    let control = Control::from_mxcsr(NEAREST);
    let mut random = Random(0x6A09_E667_F3BC_C909);
    let kinds = [
        Arithmetic::Add,
        Arithmetic::Sub,
        Arithmetic::Mul,
        Arithmetic::Div,
    ];
    let mut compared = 0;
    for _ in 0..300_000 {
        let (a, b) = (random.bits(F32), random.bits(F32));
        let (x, y) = (f32::from_bits(a as u32), f32::from_bits(b as u32));
        let hosts = [x + y, x - y, x * y, x / y];
        for (kind, host) in kinds.into_iter().zip(hosts) {
            let ours = softfloat::arithmetic(kind, F32, a, b, &control);
            if host.is_nan() {
                assert!(f32::from_bits(ours.bits as u32).is_nan());
            } else {
                assert_eq!(ours.bits as u32, host.to_bits(), "{kind:?} {a:x} {b:x}");
                compared += 1;
            }
        }
        let (a, b) = (random.bits(F64), random.bits(F64));
        let (x, y) = (f64::from_bits(a), f64::from_bits(b));
        let hosts = [x + y, x - y, x * y, x / y];
        for (kind, host) in kinds.into_iter().zip(hosts) {
            let ours = softfloat::arithmetic(kind, F64, a, b, &control);
            if host.is_nan() {
                assert!(f64::from_bits(ours.bits).is_nan());
            } else {
                assert_eq!(ours.bits, host.to_bits(), "{kind:?} {a:x} {b:x}");
                compared += 1;
            }
        }
        if x >= 0.0 {
            let ours = softfloat::square_root(F64, a, &control).bits;
            assert_eq!(ours, x.sqrt().to_bits(), "sqrt {a:x}");
        }
        let single = f32::from_bits(random.next() as u32);
        if single >= 0.0 {
            let ours = softfloat::square_root(F32, u64::from(single.to_bits()), &control).bits;
            assert_eq!(ours as u32, single.sqrt().to_bits());
        }
        if !x.is_nan() {
            let ours = softfloat::convert(F64, F32, a, &control).bits;
            assert_eq!(ours as u32, (x as f32).to_bits(), "narrow {a:x}");
        }
        if !single.is_nan() {
            let ours = softfloat::convert(F32, F64, u64::from(single.to_bits()), &control).bits;
            assert_eq!(ours, f64::from(single).to_bits());
        }
        let integer = random.next() as i64 >> (random.next() % 64);
        assert_eq!(
            softfloat::from_integer(F64, integer, &control).bits,
            (integer as f64).to_bits()
        );
        assert_eq!(
            softfloat::from_integer(F32, integer, &control).bits as u32,
            (integer as f32).to_bits()
        );
    }
    assert!(compared > 2_000_000, "{compared}");
}

#[test]
fn float_sequences_agree_between_engines() {
    let cases: Vec<(&str, Vec<u8>)> = fixture().into_iter().collect();
    let mut random = Random(0xBB67_AE85_84CA_A73B);
    for round in 0..2000 {
        let mut code = Vec::new();
        let mut text = Vec::new();
        for _ in 0..1 + random.next() % 14 {
            let case = &cases[(random.next() % cases.len() as u64) as usize];
            code.extend_from_slice(&case.1);
            text.push(case.0);
            if random.next().is_multiple_of(3) {
                code.extend_from_slice(&[0x0F, 0x92, 0xC0]); // setb al
            }
        }
        code.push(0xF4);
        let mut mxcsr = 0x1F80 | (random.next() as u32 & 0xE040);
        if random.next().is_multiple_of(8) {
            mxcsr &= !(1 << (7 + random.next() % 6));
        }
        let a = u128::from(random.bits(F64)) << 64 | u128::from(random.bits(F32)) << 32;
        let b = u128::from(random.bits(F64)) << 64 | u128::from(random.bits(F64));
        let (cpu, mut first) = load(&code, mxcsr, a, b, random.next());
        let interpreted = run(false, &cpu, &mut first, 64);
        let (_, mut second) = load(&code, mxcsr, a, b, 0);
        let translated = run(true, &cpu, &mut second, 64);
        assert_eq!(
            translated.exit_reason, interpreted.exit_reason,
            "{round}: {text:#?}"
        );
        assert_eq!(
            translated.instructions_executed,
            interpreted.instructions_executed
        );
        assert_eq!(
            translated.final_state, interpreted.final_state,
            "{round}: {text:#?}"
        );
    }
}
