// SPDX-License-Identifier: GPL-3.0-or-later
//! gfx1030 extension-word negative tests (issue #172): truncated and invalid
//! literal, SDWA, DPP16, DPP8 and MIMG NSA trailers.
//!
//! Every source instruction is original and was assembled by llvm-mc 23.1.6
//! (`source_encodings_match_llvm` re-checks the bytes). Expected results come
//! from LLVM's disassembly of the cut or altered bytes and from the field
//! layouts in AMD's public "RDNA 2 Instruction Set Architecture" reference
//! (microcode formats: SDWA, DPP16, DPP8, MIMG NSA), never from Xenora's
//! decoder. `identify` recognises lengths and allowed extensions; it does not
//! validate control-word values (see `reserved_control_values_keep_their_trailer_lengths`).

use std::io::Write;
use std::process::{Command, Stdio};

use xenora_isa::meta::{
    IdentifyError, SRC_DPP8, SRC_DPP16, SRC_LITERAL, SRC_SDWA, Trailer, VectorControl, identify,
};

struct Fixture {
    source: &'static str,
    bytes: &'static [u8],
}

const SOP2_LITERAL: Fixture = Fixture {
    source: "s_add_u32 s9, 0x12345679, s11",
    bytes: &[0xff, 0x0b, 0x09, 0x80, 0x79, 0x56, 0x34, 0x12],
};
const SOPK_IMMEDIATE: Fixture = Fixture {
    source: "s_setreg_imm32_b32 hwreg(HW_REG_MODE), 0x125678ab",
    bytes: &[0x01, 0xf8, 0x80, 0xba, 0xab, 0x78, 0x56, 0x12],
};
const VOP1_LITERAL: Fixture = Fixture {
    source: "v_mov_b32_e32 v7, 0x12345679",
    bytes: &[0xff, 0x02, 0x0e, 0x7e, 0x79, 0x56, 0x34, 0x12],
};
const VOP2_LITERAL: Fixture = Fixture {
    source: "v_add_f32_e32 v7, 0x3f800003, v8",
    bytes: &[0xff, 0x10, 0x0e, 0x06, 0x03, 0x00, 0x80, 0x3f],
};
const FMAMK: Fixture = Fixture {
    source: "v_fmamk_f32 v7, v8, 0x40000001, v9",
    bytes: &[0x08, 0x13, 0x0e, 0x58, 0x01, 0x00, 0x00, 0x40],
};
const FMAAK: Fixture = Fixture {
    source: "v_fmaak_f32 v7, v8, v9, 0x40000002",
    bytes: &[0x08, 0x13, 0x0e, 0x5a, 0x02, 0x00, 0x00, 0x40],
};
const VOPC_LITERAL: Fixture = Fixture {
    source: "v_cmp_eq_u32_e32 vcc_lo, 0x1235, v9",
    bytes: &[0xff, 0x12, 0x84, 0x7d, 0x35, 0x12, 0x00, 0x00],
};
const VOP3_LITERAL: Fixture = Fixture {
    source: "v_fma_f32 v7, v8, v9, 0x3f800003",
    bytes: &[
        0x07, 0x00, 0x4b, 0xd5, 0x08, 0x13, 0xfe, 0x03, 0x03, 0x00, 0x80, 0x3f,
    ],
};
const VOP3P_LITERAL: Fixture = Fixture {
    source: "v_pk_fma_f16 v7, v8, v9, 0x3c02",
    bytes: &[
        0x07, 0x40, 0x0e, 0xcc, 0x08, 0x13, 0xfe, 0x1b, 0x02, 0x3c, 0x00, 0x00,
    ],
};
const VOP1_SDWA: Fixture = Fixture {
    source: "v_mov_b32_sdwa v7, v8 dst_sel:BYTE_1 dst_unused:UNUSED_PAD src0_sel:DWORD",
    bytes: &[0xf9, 0x02, 0x0e, 0x7e, 0x08, 0x01, 0x06, 0x00],
};
const VOP2_SDWA: Fixture = Fixture {
    source: "v_add_f32_sdwa v7, v8, v9 dst_sel:DWORD dst_unused:UNUSED_PAD src0_sel:WORD_0 src1_sel:DWORD",
    bytes: &[0xf9, 0x12, 0x0e, 0x06, 0x08, 0x06, 0x04, 0x06],
};
const VOPC_SDWA: Fixture = Fixture {
    source: "v_cmp_eq_u32_sdwa s4, v8, v9 src0_sel:DWORD src1_sel:DWORD",
    bytes: &[0xf9, 0x12, 0x84, 0x7d, 0x08, 0x84, 0x06, 0x06],
};
const DPP16: Fixture = Fixture {
    source: "v_add_f32_dpp v7, v8, v9 row_shl:2 row_mask:0xf bank_mask:0xf",
    bytes: &[0xfa, 0x12, 0x0e, 0x06, 0x08, 0x02, 0x01, 0xff],
};
const DPP8: Fixture = Fixture {
    source: "v_mov_b32_dpp v7, v8 dpp8:[2,3,0,1,6,7,4,5]",
    bytes: &[0xe9, 0x02, 0x0e, 0x7e, 0x08, 0x1a, 0xe2, 0xb3],
};
const DPP8_FI: Fixture = Fixture {
    source: "v_mov_b32_dpp v7, v8 dpp8:[2,3,0,1,6,7,4,5] fi:1",
    bytes: &[0xea, 0x02, 0x0e, 0x7e, 0x08, 0x1a, 0xe2, 0xb3],
};
const NSA1: Fixture = Fixture {
    source: "image_sample v[4:7], [v8, v10], s[0:7], s[8:11] dmask:0xf dim:SQ_RSRC_IMG_2D",
    bytes: &[
        0x0a, 0x0f, 0x80, 0xf0, 0x08, 0x04, 0x40, 0x00, 0x0a, 0x00, 0x00, 0x00,
    ],
};
const NSA2: Fixture = Fixture {
    source: "image_sample_d v[4:7], [v8, v10, v12, v14, v16, v18], s[0:7], s[8:11] dmask:0xf dim:SQ_RSRC_IMG_2D",
    bytes: &[
        0x0c, 0x0f, 0x88, 0xf0, 0x08, 0x04, 0x40, 0x00, 0x0a, 0x0c, 0x0e, 0x10, 0x12, 0x00, 0x00,
        0x00,
    ],
};
const NSA3: Fixture = Fixture {
    source: "image_sample_c_d_cl v[4:7], [v8, v10, v12, v14, v16, v18, v20, v22, v24, v26, v28], s[0:7], s[8:11] dmask:0xf dim:SQ_RSRC_IMG_3D",
    bytes: &[
        0x16, 0x0f, 0xac, 0xf0, 0x08, 0x04, 0x40, 0x00, 0x0a, 0x0c, 0x0e, 0x10, 0x12, 0x14, 0x16,
        0x18, 0x1a, 0x1c, 0x00, 0x00,
    ],
};
const FRACT_F64: Fixture = Fixture {
    source: "v_fract_f64_e32 v[6:7], v[8:9]",
    bytes: &[0x08, 0x7d, 0x0c, 0x7e],
};
const READFIRSTLANE: Fixture = Fixture {
    source: "v_readfirstlane_b32 s7, v8",
    bytes: &[0x08, 0x05, 0x0e, 0x7e],
};
const FMAC: Fixture = Fixture {
    source: "v_fmac_f32_e32 v7, v8, v9",
    bytes: &[0x08, 0x13, 0x0e, 0x56],
};
const READLANE: Fixture = Fixture {
    source: "v_readlane_b32 s7, v8, s9",
    bytes: &[0x07, 0x00, 0x60, 0xd7, 0x08, 0x13, 0x00, 0x02],
};
const NOP: Fixture = Fixture {
    source: "s_nop 7",
    bytes: &[0x07, 0x00, 0x80, 0xbf],
};
const VOPC_REGISTER: Fixture = Fixture {
    source: "v_cmp_eq_u32_e32 vcc_lo, v8, v9",
    bytes: &[0x08, 0x13, 0x84, 0x7d],
};

#[test]
fn proper_prefixes_report_exact_required_lengths() {
    // Each row cuts at every dword boundary and one byte before the end.
    // LLVM emits no original instruction for any prefix. Below, "silent"
    // means no diagnostic, and "invalid" means LLVM's invalid encoding warning.
    // Empty input is silent in all rows. The expected values are the next
    // base boundary (4 or 8) until the base is present, then the LLVM length.
    let cases: &[(&Fixture, &[usize])] = &[
        // silent, SSRC0 selects the literal dword.
        (&SOP2_LITERAL, &[4, 8, 8]),
        // invalid, mandatory SOPK immediate.
        (&SOPK_IMMEDIATE, &[4, 8, 8]),
        // silent, literal selector.
        (&VOP1_LITERAL, &[4, 8, 8]),
        // silent, literal selector.
        (&VOP2_LITERAL, &[4, 8, 8]),
        // invalid, embedded multiplier dword.
        (&FMAMK, &[4, 8, 8]),
        // invalid, embedded addend dword.
        (&FMAAK, &[4, 8, 8]),
        // silent, literal selector.
        (&VOPC_LITERAL, &[4, 8, 8]),
        // invalid at 4 then silent, 8-byte base and literal.
        (&VOP3_LITERAL, &[4, 8, 12, 12]),
        // invalid at 4 then silent, 8-byte base and literal.
        (&VOP3P_LITERAL, &[4, 8, 12, 12]),
        // invalid, SDWA control dword.
        (&VOP1_SDWA, &[4, 8, 8]),
        // invalid, SDWA control dword.
        (&VOP2_SDWA, &[4, 8, 8]),
        // invalid, SDWAB control dword.
        (&VOPC_SDWA, &[4, 8, 8]),
        // invalid, DPP16 control dword.
        (&DPP16, &[4, 8, 8]),
        // invalid, DPP8 control dword (fi:0).
        (&DPP8, &[4, 8, 8]),
        // invalid, DPP8 control dword (fi:1).
        (&DPP8_FI, &[4, 8, 8]),
        // invalid, 8-byte base and one NSA dword.
        (&NSA1, &[4, 8, 12, 12]),
        // invalid, 8-byte base and two NSA dwords.
        // LLVM also prints a stray v_mul_f32 from the address bytes.
        (&NSA2, &[4, 8, 16, 16, 16]),
        // invalid, 8-byte base and three NSA dwords.
        // LLVM also prints stray v_mul_f32 and v_mul_hi_u32_u24 from the address bytes.
        (&NSA3, &[4, 8, 20, 20, 20, 20]),
    ];
    for &(fixture, expected) in cases {
        let len = fixture.bytes.len();
        let prefixes: Vec<_> = (0..len).step_by(4).chain([len - 1]).collect();
        assert_eq!(prefixes.len(), expected.len(), "{}", fixture.source);
        for (prefix, &needed) in prefixes.into_iter().zip(expected) {
            assert_eq!(
                identify(&fixture.bytes[..prefix]),
                Err(IdentifyError::Truncated { needed }),
                "{} at {prefix}/{len} bytes",
                fixture.source,
            );
        }
    }
}

#[test]
fn disallowed_vector_selectors_and_literal_are_rejected() {
    // Replace the nine-bit SRC0 field of each assembled e32 instruction
    // and append a known control or literal dword.
    let cases = [
        // LLVM rejects SDWA on v_fract_f64. Expect InvalidExtension("sdwa").
        (&FRACT_F64, SRC_SDWA, &VOP1_SDWA, "sdwa"),
        // LLVM rejects DPP16 on v_fract_f64. Expect InvalidExtension("dpp16").
        (&FRACT_F64, SRC_DPP16, &DPP16, "dpp16"),
        // LLVM rejects DPP8 on v_fract_f64 and prints the trailer as s_cmpk_lt_i32.
        // Expect InvalidExtension("dpp8") for the rejected first instruction.
        (&FRACT_F64, SRC_DPP8, &DPP8, "dpp8"),
        // LLVM rejects SDWA on v_fmac_f32. Expect InvalidExtension("sdwa").
        (&FMAC, SRC_SDWA, &VOP1_SDWA, "sdwa"),
        // LLVM rejects DPP16 on this v_cmp_eq_u32. Expect InvalidExtension("dpp16").
        (&VOPC_REGISTER, SRC_DPP16, &DPP16, "dpp16"),
        // LLVM prints v_readfirstlane_b32 with "Invalid immediate" because
        // this source must be a VGPR. Expect InvalidExtension("literal").
        (&READFIRSTLANE, SRC_LITERAL, &VOP1_LITERAL, "literal"),
    ];
    for (fixture, selector, trailer, reason) in cases {
        let mut bytes = fixture.bytes.to_vec();
        let word = u32::from_le_bytes(fixture.bytes.try_into().unwrap());
        bytes.copy_from_slice(&((word & !0x1ff) | selector).to_le_bytes());
        bytes.extend_from_slice(&trailer.bytes[4..]);
        assert_eq!(
            identify(&bytes),
            Err(IdentifyError::InvalidExtension(reason)),
            "{} with SRC0={selector:#x}: {bytes:02x?}",
            fixture.source,
        );
    }
}

#[test]
fn literal_for_vop3_readlane_is_rejected() {
    // v_readlane_b32: change SRC0 in the second base dword at bits 8:0
    // and append the v_mov_b32 literal. LLVM prints "Invalid immediate"
    // for the VGPR-only source. Expect the form to reject the literal.
    let mut bytes = READLANE.bytes.to_vec();
    let word = u32::from_le_bytes(bytes[4..8].try_into().unwrap());
    bytes[4..8].copy_from_slice(&((word & !0x1ff) | SRC_LITERAL).to_le_bytes());
    bytes.extend_from_slice(&VOP1_LITERAL.bytes[4..]);
    assert_eq!(
        identify(&bytes),
        Err(IdentifyError::InvalidExtension("literal"))
    );
}

/// Identification recognizes length independently of control-word validation.
#[test]
fn reserved_control_values_keep_their_trailer_lengths() {
    let cases = [
        // v_add_f32_dpp: replace DPP_CTRL 0x102 with 0x100, which LLVM reports as reserved.
        // LLVM prints "Invalid dpp_ctrl value". Expect 8 bytes and DPP16.
        (&DPP16, 5, 0, "V_ADD_F32", VectorControl::Dpp16),
        // v_mov_b32_sdwa: change SRC0_SEL from 6 to 7, which LLVM treats as reserved.
        // LLVM prints DWORD and reassembles it as 6. Expect 8 bytes and SDWA.
        (&VOP1_SDWA, 6, 7, "V_MOV_B32", VectorControl::Sdwa),
    ];
    for (fixture, offset, value, name, control) in cases {
        let mut bytes = fixture.bytes.to_vec();
        bytes[offset] = value;
        let found = identify(&bytes).unwrap();
        assert_eq!(found.form.name, name);
        assert_eq!(found.len, 8);
        assert_eq!(
            found.trailer,
            Trailer {
                vector_control: Some(control),
                ..Trailer::default()
            },
        );
    }
}

#[test]
fn next_instruction_starts_after_the_trailer() {
    // Append s_nop 7 to each original instruction. LLVM disassembles the
    // two instructions with lengths (offset, 4). Expect that boundary from
    // identify and the trailer selected by SRC0, SSRC0, or MIMG.NSA.
    let cases = [
        // LLVM emits s_nop at byte 8. Expect a scalar literal dword.
        (
            &SOP2_LITERAL,
            "S_ADD_U32",
            8,
            Trailer {
                literal: true,
                ..Trailer::default()
            },
        ),
        // LLVM emits s_nop at byte 8. Expect an SDWA control dword.
        (
            &VOP1_SDWA,
            "V_MOV_B32",
            8,
            Trailer {
                vector_control: Some(VectorControl::Sdwa),
                ..Trailer::default()
            },
        ),
        // LLVM emits s_nop at byte 8. Expect a DPP16 control dword.
        (
            &DPP16,
            "V_ADD_F32",
            8,
            Trailer {
                vector_control: Some(VectorControl::Dpp16),
                ..Trailer::default()
            },
        ),
        // LLVM emits s_nop at byte 8. Expect DPP8 with fi:0.
        (
            &DPP8,
            "V_MOV_B32",
            8,
            Trailer {
                vector_control: Some(VectorControl::Dpp8 {
                    fetch_inactive: false,
                }),
                ..Trailer::default()
            },
        ),
        // LLVM emits s_nop at byte 8. Expect DPP8 with fi:1.
        (
            &DPP8_FI,
            "V_MOV_B32",
            8,
            Trailer {
                vector_control: Some(VectorControl::Dpp8 {
                    fetch_inactive: true,
                }),
                ..Trailer::default()
            },
        ),
        // LLVM emits s_nop at byte 12. Expect one NSA dword.
        (
            &NSA1,
            "IMAGE_SAMPLE",
            12,
            Trailer {
                nsa_dwords: 1,
                ..Trailer::default()
            },
        ),
        // LLVM emits s_nop at byte 16. Expect two NSA dwords.
        (
            &NSA2,
            "IMAGE_SAMPLE_D",
            16,
            Trailer {
                nsa_dwords: 2,
                ..Trailer::default()
            },
        ),
        // LLVM emits s_nop at byte 20. Expect three NSA dwords.
        (
            &NSA3,
            "IMAGE_SAMPLE_C_D_CL",
            20,
            Trailer {
                nsa_dwords: 3,
                ..Trailer::default()
            },
        ),
    ];
    for (fixture, name, offset, trailer) in cases {
        let bytes = [fixture.bytes, NOP.bytes].concat();
        let first = identify(&bytes).unwrap();
        assert_eq!(
            (first.form.name, first.len, first.trailer),
            (name, offset, trailer),
            "{} followed by {}",
            fixture.source,
            NOP.source,
        );
        let second = identify(&bytes[first.len..]).unwrap();
        assert_eq!(
            (second.form.name, second.len, second.trailer),
            ("S_NOP", 4, Trailer::default()),
        );
        assert_eq!(first.len + second.len, bytes.len());
    }
}

#[test]
#[ignore = "requires LLVM with the gfx1030 AMDGPU target"]
fn source_encodings_match_llvm() {
    let fixtures = [
        &SOP2_LITERAL,
        &SOPK_IMMEDIATE,
        &VOP1_LITERAL,
        &VOP2_LITERAL,
        &FMAMK,
        &FMAAK,
        &VOPC_LITERAL,
        &VOP3_LITERAL,
        &VOP3P_LITERAL,
        &VOP1_SDWA,
        &VOP2_SDWA,
        &VOPC_SDWA,
        &DPP16,
        &DPP8,
        &DPP8_FI,
        &NSA1,
        &NSA2,
        &NSA3,
        &FRACT_F64,
        &READFIRSTLANE,
        &FMAC,
        &READLANE,
        &NOP,
        &VOPC_REGISTER,
    ];
    let executable = std::env::var_os("LLVM_BIN").map_or_else(
        || "llvm-mc".into(),
        |directory| std::path::PathBuf::from(directory).join("llvm-mc"),
    );
    let mut llvm = Command::new(executable)
        .args([
            "-triple=amdgcn-amd-amdhsa",
            "-mcpu=gfx1030",
            "-mattr=+wavefrontsize32",
            "-show-encoding",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("launch llvm-mc (provide its directory in LLVM_BIN or put it on PATH)");
    {
        let mut input = llvm.stdin.take().unwrap();
        for fixture in fixtures {
            writeln!(input, "{}", fixture.source).unwrap();
        }
    }
    let output = llvm.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr),
    );
    let assembly = String::from_utf8(output.stdout).unwrap();
    let encodings: Vec<_> = assembly
        .lines()
        .filter_map(|line| line.split_once("; encoding: [").map(|(_, bytes)| bytes))
        .collect();
    assert_eq!(encodings.len(), fixtures.len());
    for (fixture, encoding) in fixtures.into_iter().zip(encodings) {
        let bytes: Vec<_> = encoding
            .strip_suffix(']')
            .unwrap()
            .split(',')
            .map(|byte| u8::from_str_radix(byte.trim().strip_prefix("0x").unwrap(), 16).unwrap())
            .collect();
        assert_eq!(bytes, fixture.bytes, "{}", fixture.source);
    }
}
