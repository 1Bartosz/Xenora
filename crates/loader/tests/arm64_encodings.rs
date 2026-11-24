// SPDX-License-Identifier: GPL-3.0-or-later

//! Assembler encodings against LLVM (`clang -target arm64-apple-macos -c`, read
//! back with `objdump -d`).

use xenora_loader::Arm64Assembler;
use xenora_loader::arm64::Condition;

type Emit = fn(&mut Arm64Assembler);

#[test]
fn flag_and_byte_operations_match_llvm() {
    let cases: &[(&str, Emit, u32)] = &[
        ("cset w0, hs", |a| a.cset_32(0, Condition::Cs), 0x1a9f37e0),
        ("cset w12, vs", |a| a.cset_32(12, Condition::Vs), 0x1a9f77ec),
        ("cset w30, eq", |a| a.cset_32(30, Condition::Eq), 0x1a9f17fe),
        (
            "csel x9, x10, x11, ne",
            |a| a.csel_64(9, 10, 11, Condition::Ne),
            0x9a8b1149,
        ),
        (
            "csel x0, x30, x17, lt",
            |a| a.csel_64(0, 30, 17, Condition::Lt),
            0x9a91b3c0,
        ),
        (
            "strb w0, [x19, #136]",
            |a| a.strb_uoff(0, 19, 136),
            0x39022260,
        ),
        (
            "strb wzr, [x19, #141]",
            |a| a.strb_uoff(31, 19, 141),
            0x3902367f,
        ),
        (
            "ldrb w12, [x19, #140]",
            |a| a.ldrb_uoff(12, 19, 140),
            0x3942326c,
        ),
        (
            "eor w12, w11, w10, lsr #4",
            |a| a.eor_lsr_32(12, 11, 10, 4),
            0x4a4a116c,
        ),
        (
            "eor w1, w2, w3, lsr #0",
            |a| a.eor_lsr_32(1, 2, 3, 0),
            0x4a430041,
        ),
        (
            "orr w12, w13, w14",
            |a| a.orr_reg_32(12, 13, 14),
            0x2a0e01ac,
        ),
        (
            "bic w12, w13, w14",
            |a| a.bic_reg_32(12, 13, 14),
            0x0a2e01ac,
        ),
        ("mvn x11, x9", |a| a.mvn_64(11, 9), 0xaa2903eb),
        ("mvn x0, x30", |a| a.mvn_64(0, 30), 0xaa3e03e0),
        (
            "ubfx w12, w13, #4, #1",
            |a| a.ubfx_bit_32(12, 13, 4),
            0x530411ac,
        ),
        ("rev x10, x9", |a| a.rev(10, 9, true), 0xdac00d2a),
        ("rev w0, w30", |a| a.rev(0, 30, false), 0x5ac00bc0),
        ("lsr x0, x0, #8", |a| a.lsr_imm_64(0, 0, 8), 0xd348fc00),
        ("lsr x9, x30, #63", |a| a.lsr_imm_64(9, 30, 63), 0xd37fffc9),
    ];
    for (text, emit, word) in cases {
        let mut assembler = Arm64Assembler::new();
        emit(&mut assembler);
        assert_eq!(assembler.instructions(), [*word], "{text}");
    }
}
