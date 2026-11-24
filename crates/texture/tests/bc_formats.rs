// SPDX-License-Identifier: GPL-3.0-or-later
//! Expected values are worked by hand from the Microsoft BC2-BC5 and Khronos
//! S3TC/RGTC formulas (truncating integer division); the production decoder is
//! never used to produce them.

use xenora_texture::{
    decode_bc2, decode_bc3, decode_bc4_snorm, decode_bc4_unorm, decode_bc5_snorm, decode_bc5_unorm,
};

/// Packs sixteen 3-bit indices bit by bit, texel 0 in the lowest bit.
fn pack3(indices: [u8; 16]) -> [u8; 6] {
    let mut out = [0u8; 6];
    for (t, idx) in indices.iter().enumerate() {
        for bit in 0..3 {
            if (idx >> bit) & 1 == 1 {
                let pos = 3 * t + bit;
                out[pos / 8] |= 1 << (pos % 8);
            }
        }
    }
    out
}

fn bc4(a0: u8, a1: u8, indices: [u8; 16]) -> [u8; 8] {
    let i = pack3(indices);
    [a0, a1, i[0], i[1], i[2], i[3], i[4], i[5]]
}

const SEQ: [u8; 16] = [0, 1, 2, 3, 4, 5, 6, 7, 0, 1, 2, 3, 4, 5, 6, 7];

fn expect(palette: [u8; 8]) -> [u8; 16] {
    std::array::from_fn(|t| palette[t % 8])
}

#[test]
fn packing_helper_matches_hand_bytes() {
    // 111 110 101 100 011 010 001 000 = 0xFAC688, twice.
    assert_eq!(pack3(SEQ), [0x88, 0xC6, 0xFA, 0x88, 0xC6, 0xFA]);
}

#[test]
fn bc4_unorm_eight_value_mode() {
    // a0 > a1: p[k+1] = ((7-k)*200 + k*100) / 7 for k = 1..6.
    // 1300/7=185, 1200/7=171, 1100/7=157, 1000/7=142, 900/7=128, 800/7=114.
    let out = decode_bc4_unorm(bc4(200, 100, SEQ));
    assert_eq!(out, expect([200, 100, 185, 171, 157, 142, 128, 114]));
}

#[test]
fn bc4_unorm_six_value_mode_with_zero_and_255() {
    // a0 <= a1: ((5-k)*100 + k*200) / 5: 600/5=120, 700/5=140, 800/5=160,
    // 900/5=180, then 0 and 255.
    let out = decode_bc4_unorm(bc4(100, 200, SEQ));
    assert_eq!(out, expect([100, 200, 120, 140, 160, 180, 0, 255]));
    // Equal endpoints also take the six-value mode.
    let out = decode_bc4_unorm(bc4(50, 50, SEQ));
    assert_eq!(out, expect([50, 50, 50, 50, 50, 50, 0, 255]));
}

#[test]
fn bc4_snorm_eight_value_mode_truncates_toward_zero() {
    // a0=100 > a1=-100: 500/7=71, 300/7=42, 100/7=14, -100/7=-14,
    // -300/7=-42, -500/7=-71.
    let b = bc4(100, 0x9C, SEQ); // 0x9C = -100
    let out = decode_bc4_snorm(b);
    let e: [i8; 8] = [100, -100, 71, 42, 14, -14, -42, -71];
    assert_eq!(out, std::array::from_fn(|t| e[t % 8]));
}

#[test]
fn bc4_snorm_six_value_mode_with_fixed_extremes() {
    // a0=-100 <= a1=100: (-400+100)/5=-60, (-300+200)/5=-20, 20, 60, -127, 127.
    let out = decode_bc4_snorm(bc4(0x9C, 100, SEQ));
    let e: [i8; 8] = [-100, 100, -60, -20, 20, 60, -127, 127];
    assert_eq!(out, std::array::from_fn(|t| e[t % 8]));
}

#[test]
fn bc4_snorm_minus_128_is_kept_for_endpoints_and_acts_as_minus_127() {
    // a0=-128 < a1=127: six-value mode, x=-127: p2=(4*-127+127)/5=-381/5=-76.
    let out = decode_bc4_snorm(bc4(0x80, 127, SEQ));
    assert_eq!(out[0], -128);
    assert_eq!(out[1], 127);
    assert_eq!(out[2], -76);
    // a0=127 > a1=-128: eight-value mode, y=-127: p2=(6*127-127)/7=635/7=90.
    let out = decode_bc4_snorm(bc4(127, 0x80, SEQ));
    assert_eq!(out[1], -128);
    assert_eq!(out[2], 90);
}

#[test]
fn every_index_position_is_read_from_its_own_bits() {
    // Texel n alone uses index 1 (value 100); all others index 0 (value 200).
    // Texels 2, 5, 10 and 13 span byte boundaries.
    for n in 0..16 {
        let mut idx = [0u8; 16];
        idx[n] = 1;
        let out = decode_bc4_unorm(bc4(200, 100, idx));
        for (t, v) in out.iter().enumerate() {
            assert_eq!(*v, if t == n { 100 } else { 200 }, "texel {t}, set {n}");
        }
    }
}

#[test]
fn wrong_index_bit_order_would_fail() {
    // Texel 5 occupies bits 15..17 (byte 1 bit 7, byte 2 bits 0-1); texel 10
    // occupies bits 30..32 (byte 3 bits 6-7, byte 4 bit 0).
    let mut idx = [0u8; 16];
    idx[5] = 1;
    idx[10] = 2;
    let b = bc4(200, 100, idx);
    // Hand-checked bytes: bit 15 set; bit 31 set.
    assert_eq!(&b[2..], &[0x00, 0x80, 0x00, 0x80, 0x00, 0x00]);
    let out = decode_bc4_unorm(b);
    assert_eq!(out[5], 100);
    assert_eq!(out[10], 185);
    assert_ne!(out[5], out[10]);
    // A decoder reading the 48 bits most-significant first (wrong order)
    // lands on different texels; show that this block tells them apart.
    let bits = b[2..8]
        .iter()
        .rev()
        .fold(0u64, |a, x| (a << 8) | u64::from(*x));
    let wrong = |t: usize| ((bits >> (45 - 3 * t)) & 7) as u8;
    assert_ne!(wrong(5), 1);
    assert_ne!(wrong(10), 2);
}

fn colour_bytes(c0: u16, c1: u16, indices: u32) -> [u8; 8] {
    let (a, b, i) = (c0.to_le_bytes(), c1.to_le_bytes(), indices.to_le_bytes());
    [a[0], a[1], b[0], b[1], i[0], i[1], i[2], i[3]]
}

#[test]
fn bc2_expands_four_bit_alpha_and_always_uses_four_colours() {
    // Red 0xF800 = (255,0,0), blue 0x001F = (0,0,255).
    // p2 = (170,0,85), p3 = (85,0,170). Texels 0..3 use indices 0..3.
    let alpha = [0x10, 0x32, 0x54, 0x76, 0x98, 0xBA, 0xDC, 0xFE];
    for (c0, c1, mid) in [
        (0xF800, 0x001F, [[170, 0, 85], [85, 0, 170]]),
        // Endpoints swapped: still four-colour mode, no transparent index.
        (0x001F, 0xF800, [[85, 0, 170], [170, 0, 85]]),
    ] {
        let mut block = [0u8; 16];
        block[..8].copy_from_slice(&alpha);
        block[8..].copy_from_slice(&colour_bytes(c0, c1, 0b11_10_01_00));
        let out = decode_bc2(block);
        let first = if c0 == 0xF800 {
            [255, 0, 0]
        } else {
            [0, 0, 255]
        };
        let second = if c0 == 0xF800 {
            [0, 0, 255]
        } else {
            [255, 0, 0]
        };
        let rgb = [first, second, mid[0], mid[1]];
        for t in 0..16 {
            assert_eq!(out[t][3], t as u8 * 17, "alpha texel {t}");
            if t < 4 {
                assert_eq!(out[t][..3], rgb[t], "colour texel {t}");
            } else {
                assert_eq!(out[t][..3], first, "colour texel {t}");
            }
        }
    }
}

#[test]
fn bc3_uses_bc4_style_alpha_and_four_colour_block() {
    let mut block = [0u8; 16];
    block[..8].copy_from_slice(&bc4(200, 100, SEQ));
    // c0 < c1 would be 3-colour in BC1; index 3 must stay interpolated.
    block[8..].copy_from_slice(&colour_bytes(0x001F, 0xF800, 0xFFFF_FFFF));
    let out = decode_bc3(block);
    let alpha = expect([200, 100, 185, 171, 157, 142, 128, 114]);
    for t in 0..16 {
        assert_eq!(out[t][3], alpha[t]);
        assert_eq!(out[t][..3], [170, 0, 85]);
    }
    // Six-value alpha mode.
    block[..8].copy_from_slice(&bc4(100, 200, SEQ));
    let alpha = expect([100, 200, 120, 140, 160, 180, 0, 255]);
    let out = decode_bc3(block);
    for t in 0..16 {
        assert_eq!(out[t][3], alpha[t]);
    }
}

#[test]
fn bc5_channel_order_is_red_then_green() {
    // All texels index 1 = 0b001 repeated: bytes 0x49 0x92 0x24 twice.
    let ones = [0x49, 0x92, 0x24, 0x49, 0x92, 0x24];
    let mut block = [0u8; 16];
    block[..2].copy_from_slice(&[200, 100]);
    block[2..8].copy_from_slice(&ones);
    block[8..10].copy_from_slice(&[100, 200]);
    block[10..].copy_from_slice(&ones);
    for texel in decode_bc5_unorm(block) {
        assert_eq!(texel, [100, 200]);
    }
}

#[test]
fn bc5_snorm_channels_and_fixed_values() {
    // Index 6 = 0b110 repeated: bytes 0xB6 0x6D 0xDB twice.
    let sixes = [0xB6, 0x6D, 0xDB, 0xB6, 0x6D, 0xDB];
    let mut block = [0u8; 16];
    // Red: a0=100 > a1=-100, index 6 = -42. Green: a0=-100 <= a1=100, index 6 = -127.
    block[..2].copy_from_slice(&[100, 0x9C]);
    block[2..8].copy_from_slice(&sixes);
    block[8..10].copy_from_slice(&[0x9C, 100]);
    block[10..].copy_from_slice(&sixes);
    for texel in decode_bc5_snorm(block) {
        assert_eq!(texel, [-42, -127]);
    }
}
