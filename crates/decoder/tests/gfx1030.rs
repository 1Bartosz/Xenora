use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use xenora_decoder::{
    Architecture, DecodeError, DecodeErrorKind, DecodedInstruction, Instruction, VectorSource,
    decode,
};

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
        assert!(matches!(
            error.kind,
            DecodeErrorKind::UnsupportedForm { mnemonic: actual, .. } if actual == *mnemonic
        ));
    }

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

#[test]
#[ignore = "requires LLVM with the gfx1030 AMDGPU target"]
fn compares_fixture_instructions_against_llvm() {
    // Also covered by the independent reference decoder's high-register test.
    let high_registers: &[u8] = &[
        0xfe, 0x03, 0xfe, 0x7f, 0xff, 0, 0x25, 0xd5, 0x80, 0xff, 0x03, 0x02, 0, 0, 0x81, 0xbf,
    ];
    for fixture in [FIXTURE, BITWISE_FIXTURE, FLOAT_FIXTURE, high_registers] {
        compare_against_llvm(fixture);
    }
}

fn compare_against_llvm(fixture: &[u8]) {
    let executable = std::env::var_os("LLVM_BIN").map_or_else(
        || PathBuf::from("llvm-mc"),
        |directory| PathBuf::from(directory).join("llvm-mc"),
    );
    let mut child = Command::new(executable)
        .args([
            "-disassemble",
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
    {
        let mut input = child.stdin.take().unwrap();
        for byte in fixture {
            write!(input, "0x{byte:02x} ").unwrap();
        }
        writeln!(input).unwrap();
    }

    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success() && output.stderr.is_empty(),
        "llvm-mc failed or emitted diagnostics: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    let llvm: Vec<_> = stdout
        .lines()
        .filter_map(|line| line.split_once("; encoding: ["))
        .collect();
    let xenora = decode(Architecture::Gfx1030, fixture).unwrap();
    assert_eq!(llvm.len(), xenora.len());

    let mut llvm_offset = 0;
    for ((assembly, encoding), decoded) in llvm.into_iter().zip(xenora) {
        let llvm_bytes: Vec<_> = encoding
            .trim_end_matches(']')
            .split(',')
            .map(|byte| u8::from_str_radix(byte.trim().trim_start_matches("0x"), 16).unwrap())
            .collect();
        let llvm_end = llvm_offset + llvm_bytes.len();
        assert_eq!(decoded.offset, llvm_offset);
        assert_eq!(usize::from(decoded.byte_len), llvm_bytes.len());
        assert_eq!(llvm_bytes, fixture[llvm_offset..llvm_end]);
        let assembly = normalize_llvm_assembly(assembly.trim(), llvm_end);
        assert_eq!(decoded.instruction.to_string(), assembly);
        eprintln!("{decoded} ({} bytes)", decoded.byte_len);
        llvm_offset = llvm_end;
    }
    assert_eq!(llvm_offset, fixture.len());
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
                "0" => "0x0",
                "1.0" => "0x3f800000",
                _ => source,
            };
            format!("{mnemonic} {destination}, {normalized}")
        }
        _ => assembly.to_owned(),
    }
}
