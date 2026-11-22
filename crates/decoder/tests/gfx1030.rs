use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use xenora_decoder::{
    Architecture, DecodeError, DecodeErrorKind, DecodedInstruction, Instruction, ScalarSource,
    VectorSource, decode,
};
use xenora_ref::{Op, Source, VectorAlu};

const FIXTURE: &[u8] = include_bytes!("../../../fixtures/rdna2/add.bin");
const BITWISE_FIXTURE: &[u8] = include_bytes!("../../../fixtures/rdna2/bitwise-branch.bin");
const FLOAT_FIXTURE: &[u8] = include_bytes!("../../../fixtures/rdna2/float-exact.bin");

#[test]
fn decodes_each_fixture_instruction_with_offset_and_length() {
    let instructions = decode(Architecture::Gfx1030, FIXTURE).unwrap();
    let wanted = [
        (
            0,
            8,
            Instruction::BufferLoadDword {
                data: 1,
                index: 0,
                resource: 0,
            },
        ),
        (8, 4, Instruction::WaitLoad),
        (
            12,
            8,
            Instruction::MoveB32 {
                dst: 2,
                src: VectorSource::Immediate(0x1234_5678),
            },
        ),
        (
            20,
            8,
            Instruction::AddNoCarryU32 {
                dst: 1,
                src0: 1,
                src1: 2,
            },
        ),
        (
            28,
            8,
            Instruction::BufferStoreDword {
                data: 1,
                index: 0,
                resource: 4,
            },
        ),
        (36, 4, Instruction::WaitStore),
        (40, 4, Instruction::EndProgram),
    ]
    .map(|(offset, byte_len, instruction)| DecodedInstruction {
        offset,
        byte_len,
        instruction,
    });
    assert_eq!(instructions, wanted);
}

#[test]
fn every_non_boundary_prefix_reports_the_incomplete_instruction() {
    let lengths = [8, 4, 8, 8, 8, 4, 4];
    let mut offset = 0;
    for len in lengths {
        for available in 1..len {
            let prefix = &FIXTURE[..offset + usize::from(available)];
            let needed = if available < 4 { 4 } else { len };
            assert_eq!(
                decode(Architecture::Gfx1030, prefix).unwrap_err(),
                DecodeError {
                    offset,
                    kind: DecodeErrorKind::Truncated {
                        needed,
                        available: usize::from(available),
                    },
                }
            );
            assert_eq!(xenora_ref::decode(prefix).unwrap_err().offset, offset);
        }
        offset += usize::from(len);
    }
}

#[test]
fn rejects_other_opcodes_and_noncanonical_operands() {
    let cases: &[(&[u32], &str)] = &[
        (&[0xe030_0000, 0x8000_0100], "buffer_load_dword"),
        (&[0xe070_2000, 0x8101_0100], "buffer_store_dword"),
        (&[0xe070_2000, 0x801b_0100], "buffer_store_dword"),
        (&[0xbf8c_3f71], "s_waitcnt"),
        (&[0xbbfd_0001], "s_waitcnt_vscnt"),
        (&[0x7e04_0200], "v_mov_b32_e32"),
        (&[0x7e04_02fa, 0], "v_mov_b32_e32"),
        (&[0xd525_8001, 0x0202_0501], "v_add_nc_u32_e64"),
        (&[0xd525_0001, 0x0202_0480], "v_add_nc_u32_e64"),
        (&[0xbf81_0001], "s_endpgm"),
        (&[0xbefe_0387], "s_mov_b32"),
        (&[0xbf06_87fe], "s_cmp_eq_u32"),
        (&[0xbf06_ffff, 0], "s_cmp_eq_u32"),
        (&[0xbf82_ffff], "s_branch"),
        (&[0xbf84_fffe], "s_cbranch_scc0"),
        (&[0x3602_0680], "v_and_b32_e32"),
        (&[0x1002_06f2], "v_mul_f32_e32"),
    ];
    for (words, mnemonic) in cases {
        let bytes: Vec<_> = words.iter().flat_map(|word| word.to_le_bytes()).collect();
        let error = decode(Architecture::Gfx1030, &bytes).unwrap_err();
        assert_eq!(error.offset, 0);
        assert_eq!(xenora_ref::decode(&bytes).unwrap_err().offset, 0);
        assert!(matches!(
            error.kind,
            DecodeErrorKind::UnsupportedForm { mnemonic: actual, .. } if actual == *mnemonic
        ));
    }

    assert_eq!(xenora_ref::decode(&[0xff; 4]).unwrap_err().offset, 0);
    assert_eq!(
        decode(Architecture::Gfx1030, &[0xff; 4]).unwrap_err().kind,
        DecodeErrorKind::UnsupportedOpcode { word: 0xffff_ffff }
    );
}

#[test]
fn rejects_targets_at_eof_or_inside_a_literal() {
    for (words, target) in [
        ([0xbf82_0002u32, 0x7e04_02ff, 0x1234_5678], 12),
        ([0xbf84_0001u32, 0x7e04_02ff, 0x1234_5678], 8),
    ] {
        let bytes: Vec<_> = words.iter().flat_map(|word| word.to_le_bytes()).collect();
        assert_eq!(
            decode(Architecture::Gfx1030, &bytes).unwrap_err(),
            DecodeError {
                offset: 0,
                kind: DecodeErrorKind::InvalidBranchTarget { target },
            }
        );
    }
}

#[test]
fn accepts_zero_displacement_to_the_following_instruction() {
    let bytes = [0, 0, 0x82, 0xbf, 0, 0, 0x81, 0xbf];
    let instructions = decode(Architecture::Gfx1030, &bytes).unwrap();
    assert_eq!(
        instructions[0].instruction,
        Instruction::Branch { target: 4 }
    );
}

#[test]
fn scalar_sources_keep_literal_width_and_register_class() {
    // s_mov_b32 s10, 0x11223344 and s_cmp_eq_u32 s10, 0x11223344.
    let words = [0xbe8a_03ffu32, 0x1122_3344, 0xbf06_ff0a, 0x1122_3344];
    let bytes: Vec<_> = words.iter().flat_map(|word| word.to_le_bytes()).collect();
    let decoded = decode(Architecture::Gfx1030, &bytes).unwrap();
    assert_eq!(decoded[0].byte_len, 8);
    assert_eq!(decoded[1].offset, 8);
    assert_eq!(decoded[1].byte_len, 8);
    assert_eq!(
        decoded[1].instruction.to_string(),
        "s_cmp_eq_u32 s10, 0x11223344"
    );
    assert_eq!(
        decode(Architecture::Gfx1030, &bytes[..12]).unwrap_err(),
        DecodeError {
            offset: 8,
            kind: DecodeErrorKind::Truncated {
                needed: 8,
                available: 4
            },
        }
    );
}

fn reference_source(vector: VectorSource) -> Source {
    match vector {
        VectorSource::Register(reg) => Source::Register(reg),
        VectorSource::Immediate(bits) => Source::Immediate(bits),
    }
}

fn reference_scalar(scalar: ScalarSource) -> Source {
    match scalar {
        ScalarSource::Register(reg) => Source::Register(reg),
        ScalarSource::Immediate(bits) => Source::Immediate(bits),
    }
}

fn reference_op(instruction: Instruction) -> Op {
    match instruction {
        Instruction::BufferLoadDword {
            data,
            index,
            resource,
        } => Op::Load {
            data,
            index,
            resource,
        },
        Instruction::BufferStoreDword {
            data,
            index,
            resource,
        } => Op::Store {
            data,
            index,
            resource,
        },
        Instruction::WaitLoad => Op::WaitRead,
        Instruction::WaitStore => Op::WaitWrite,
        Instruction::MoveB32 { dst, src } => Op::Move(dst, reference_source(src)),
        Instruction::AddNoCarryU32 { dst, src0, src1 } => Op::Add(dst, src0, src1),
        Instruction::ScalarMoveB32 { dst, src } => Op::ScalarMove(dst, reference_scalar(src)),
        Instruction::ScalarCompareEqU32 { src0, src1 } => {
            Op::CompareEqual(reference_scalar(src0), reference_scalar(src1))
        }
        Instruction::BranchSccZero { target } => Op::Branch {
            target,
            if_scc_zero: true,
        },
        Instruction::Branch { target } => Op::Branch {
            target,
            if_scc_zero: false,
        },
        Instruction::VectorAndB32 { dst, src0, src1 } => {
            Op::Vector(VectorAlu::And, dst, src0, src1)
        }
        Instruction::VectorXorB32 { dst, src0, src1 } => {
            Op::Vector(VectorAlu::Xor, dst, src0, src1)
        }
        Instruction::VectorAddF32 { dst, src0, src1 } => {
            Op::Vector(VectorAlu::FloatAdd, dst, src0, src1)
        }
        Instruction::VectorMulF32 { dst, src0, src1 } => {
            Op::Vector(VectorAlu::FloatMultiply, dst, src0, src1)
        }
        Instruction::EndProgram => Op::End,
    }
}

#[test]
#[ignore = "requires LLVM with the gfx1030 AMDGPU target"]
fn compares_fixture_instructions_against_llvm() {
    let high_registers: &[u8] = &[
        0xfe, 0x03, 0xfe, 0x7f, 0xff, 0, 0x25, 0xd5, 0x80, 0xff, 0x03, 0x02, 0, 0, 0x81, 0xbf,
    ];
    let matrix = assemble_matrix();
    let mut forms = std::collections::BTreeSet::new();
    let mut count = 0;
    for fixture in [
        FIXTURE,
        BITWISE_FIXTURE,
        FLOAT_FIXTURE,
        high_registers,
        &matrix,
    ] {
        count += compare_against_llvm(fixture, &mut forms);
    }
    assert_eq!(forms.len(), 15, "forms compared: {forms:?}");
    eprintln!(
        "{count} three-way instruction comparisons across {} forms",
        forms.len()
    );
}

fn assemble_matrix() -> Vec<u8> {
    // Cover register banks and extrema, immediate classes in both scalar
    // compare positions, zero displacement and branches over literal words.
    let mut assembly = String::from(
        "
        buffer_load_dword v255, v128, s[100:103], 0 idxen
        buffer_store_dword v128, v255, s[96:99], 0 idxen
        v_and_b32_e32 v255, v128, v254
        v_xor_b32_e32 v128, v255, v127
        v_add_f32_e32 v255, v254, v128
        v_mul_f32_e32 v128, v127, v255
        s_mov_b32 s105, s104
        s_mov_b32 s0, 0xdeadbeef
        s_cmp_eq_u32 s105, s0
        s_cmp_eq_u32 s105, 0xdeadbeef
        s_cmp_eq_u32 0xdeadbeef, s105
        s_cmp_eq_u32 -16, 64
        s_cmp_eq_u32 -16, s105
        s_cmp_eq_u32 s105, -16
        s_cmp_eq_u32 -16, 0xdeadbeef
        s_cmp_eq_u32 0xdeadbeef, -16
        s_cbranch_scc0 0
        s_branch 2
        v_mov_b32_e32 v128, 0x12345678
        v_mov_b32_e32 v255, lit(0x1)
        s_mov_b32 s105, lit(0x1)
        s_cmp_eq_u32 s105, lit(0x1)
    ",
    );
    use std::fmt::Write as _;
    for value in -16..=64 {
        writeln!(assembly, "v_mov_b32_e32 v255, {value}").unwrap();
        writeln!(assembly, "s_mov_b32 s105, {value}").unwrap();
        writeln!(assembly, "s_cmp_eq_u32 {value}, {value}").unwrap();
    }
    assembly.push_str("s_endpgm\n");
    let output = llvm_mc(&[], assembly.as_bytes());
    assert_llvm_success(&output);
    let stdout = String::from_utf8(output.stdout).unwrap();
    let encodings: Vec<_> = stdout
        .lines()
        .filter_map(|line| line.split_once("; encoding: ["))
        .map(|(_, encoding)| encoding_bytes(encoding))
        .collect();
    assert_eq!(
        encodings.len(),
        assembly
            .lines()
            .filter(|line| !line.trim().is_empty())
            .count(),
        "LLVM must encode every matrix row"
    );
    encodings.into_iter().flatten().collect()
}

fn encoding_bytes(encoding: &str) -> Vec<u8> {
    encoding
        .trim_end_matches(']')
        .split(',')
        .map(|byte| u8::from_str_radix(byte.trim().trim_start_matches("0x"), 16).unwrap())
        .collect()
}

fn llvm_mc(mode: &[&str], input: &[u8]) -> std::process::Output {
    let executable = std::env::var_os("LLVM_BIN").map_or_else(
        || PathBuf::from("llvm-mc"),
        |directory| PathBuf::from(directory).join("llvm-mc"),
    );
    let mut child = Command::new(executable)
        .args(mode)
        .args([
            "-show-encoding",
            "-triple=amdgcn-amd-amdhsa",
            "-mcpu=gfx1030",
            "-mattr=+wavefrontsize32",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("launch llvm-mc (provide its directory in LLVM_BIN or put it on PATH)");
    child.stdin.take().unwrap().write_all(input).unwrap();
    child.wait_with_output().unwrap()
}

fn assert_llvm_success(output: &std::process::Output) {
    assert!(
        output.status.success() && output.stderr.is_empty(),
        "llvm-mc failed or emitted diagnostics: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn compare_against_llvm(
    fixture: &[u8],
    forms: &mut std::collections::BTreeSet<&'static str>,
) -> usize {
    let output = llvm_disassemble(fixture);
    assert_llvm_success(&output);
    let stdout = String::from_utf8(output.stdout).unwrap();
    let llvm: Vec<_> = stdout
        .lines()
        .filter_map(|line| line.split_once("; encoding: ["))
        .collect();
    let xenora = decode(Architecture::Gfx1030, fixture).unwrap();
    assert_eq!(llvm.len(), xenora.len());

    let reference = xenora_ref::decode(fixture).unwrap();
    let count = llvm.len();
    assert_eq!(count, reference.len());
    let mut llvm_offset = 0;
    for (((assembly, encoding), decoded), reference) in llvm.into_iter().zip(xenora).zip(reference)
    {
        let llvm_bytes = encoding_bytes(encoding);
        let llvm_end = llvm_offset + llvm_bytes.len();
        assert_eq!(decoded.offset, llvm_offset);
        assert_eq!(reference.offset, llvm_offset);
        assert_eq!(usize::from(decoded.byte_len), llvm_bytes.len());
        assert_eq!(reference.byte_len, llvm_bytes.len());
        assert_eq!(llvm_bytes, fixture[llvm_offset..llvm_end]);
        let assembly = normalize_llvm_assembly(assembly.trim(), llvm_end);
        assert_eq!(
            decoded.instruction.to_string(),
            assembly,
            "LLVM at {llvm_offset:#x}"
        );
        assert_eq!(
            reference.op,
            reference_op(decoded.instruction),
            "reference at {llvm_offset:#x}"
        );
        forms.insert(decoded.instruction.mnemonic());
        llvm_offset = llvm_end;
    }
    assert_eq!(llvm_offset, fixture.len());
    count
}

fn llvm_disassemble(fixture: &[u8]) -> std::process::Output {
    let input = fixture
        .iter()
        .map(|byte| format!("0x{byte:02x} "))
        .collect::<String>();
    llvm_mc(&["-disassemble"], input.as_bytes())
}

#[test]
#[ignore = "requires LLVM with the gfx1030 AMDGPU target"]
fn distinguishes_llvm_invalid_encodings_from_xenora_subset_rejections() {
    let malformed: &[&[u32]] = &[&[0xffff_ffff], &[0xe030_2000], &[0xd525_0001]];
    let outside_subset: &[&[u32]] = &[
        &[0x7e04_0200],                                        // VOP1 SGPR source
        &[0x3602_0680],                                        // VOP2 inline source
        &[0xbf8c_3f71],                                        // Different wait counter
        &[0xbf82_ffff],                                        // Backward branch
        &[0xbf82_0001, 0xbf81_0000],                           // Branch to EOF
        &[0xbf84_0001, 0x7e04_02ff, 0x1234_5678, 0xbf81_0000], // Branch into literal
    ];
    for (words, llvm_accepts) in malformed
        .iter()
        .map(|words| (words, false))
        .chain(outside_subset.iter().map(|words| (words, true)))
    {
        let bytes: Vec<_> = words.iter().flat_map(|word| word.to_le_bytes()).collect();
        assert_eq!(decode(Architecture::Gfx1030, &bytes).unwrap_err().offset, 0);
        assert_eq!(xenora_ref::decode(&bytes).unwrap_err().offset, 0);
        let output = llvm_disassemble(&bytes);
        if llvm_accepts {
            assert_llvm_success(&output);
            assert!(!output.stdout.is_empty());
        } else {
            assert!(
                String::from_utf8_lossy(&output.stderr).contains("invalid instruction encoding")
            );
            assert!(output.stdout.is_empty());
        }
    }
    eprintln!(
        "{} malformed and {} out-of-subset LLVM probes",
        malformed.len(),
        outside_subset.len()
    );
}

fn literal_bits(text: &str) -> Option<u32> {
    let digits = text
        .strip_prefix("lit(")?
        .strip_suffix(')')?
        .strip_prefix("0x")?;
    u32::from_str_radix(digits, 16).ok()
}

fn normalize_llvm_assembly(assembly: &str, next_offset: usize) -> String {
    let (mnemonic, operands) = assembly.split_once(' ').unwrap_or((assembly, ""));
    match mnemonic {
        "s_waitcnt_vscnt" if operands == "null, 0x0" => format!("{mnemonic} null, 0"),
        "s_branch" | "s_cbranch_scc0" => {
            let displacement: usize = operands.parse().unwrap();
            let target = next_offset + displacement * 4;
            format!("{mnemonic} -> 0x{target:04x}")
        }
        "v_mov_b32_e32" => {
            let (destination, source) = operands.split_once(", ").unwrap();
            let normalized = match source {
                "1.0" => "0x3f800000".to_owned(),
                literal if literal_bits(literal).is_some() => {
                    format!("0x{:x}", literal_bits(literal).unwrap())
                }
                _ => source.parse::<i32>().map_or_else(
                    |_| source.to_owned(),
                    |value| format!("0x{:x}", value as u32),
                ),
            };
            format!("{mnemonic} {destination}, {normalized}")
        }
        "s_mov_b32" | "s_cmp_eq_u32" => {
            let (left, right) = operands.split_once(", ").unwrap();
            let scalar = |operand: &str| match literal_bits(operand) {
                Some(bits) => match bits as i32 {
                    inline @ -16..=64 => inline.to_string(),
                    _ => format!("0x{bits:x}"),
                },
                None => operand.to_owned(),
            };
            format!("{mnemonic} {}, {}", scalar(left), scalar(right))
        }
        _ => assembly.to_owned(),
    }
}
