// SPDX-License-Identifier: GPL-3.0-or-later
//! BC7 tests. Expected values are worked by hand from the Khronos Data Format
//! Specification 1.3, section 20.1 (Table 109 parameters, Table 111 field
//! order, P-bits and bit replication, anchor indices, Equation 2 with the
//! Table 119 weights, Table 120 rotation). Blocks are packed by a test-local
//! bit writer; the production decoder never produces an expected value.
//!
//! Equation 2 rounding used below: `((64 - w) * e0 + w * e1 + 32) >> 6`.
//! With e0 = 0 and e1 = 255 the 2-bit weights give 0, 84, 171, 255; the 3-bit
//! weights give 0, 36, 72, 108, 147, 183, 219, 255; the 4-bit weights give
//! 0, 16, 36, 52, 68, 84, 104, 120, 135, 151, 171, 187, 203, 219, 239, 255.

use xenora_texture::{BC7_SUPPORTED_MODES, bc7_mode, decode_bc7};

/// Packs fields from bit 0 upward, starting with the mode's unary bits.
struct Block {
    bits: u128,
    pos: u32,
}

impl Block {
    fn new(mode: u32) -> Self {
        Block {
            bits: 1 << mode,
            pos: mode + 1,
        }
    }
    fn put(&mut self, value: u32, n: u32) -> &mut Self {
        assert!(
            n == 32 || value < (1 << n),
            "value {value} wider than {n} bits"
        );
        self.bits |= u128::from(value) << self.pos;
        self.pos += n;
        self
    }
    fn finish(&self) -> [u8; 16] {
        assert_eq!(self.pos, 128, "fields must fill the block");
        self.bits.to_le_bytes()
    }
}

const W4: [u8; 16] = [
    0, 16, 36, 52, 68, 84, 104, 120, 135, 151, 171, 187, 203, 219, 239, 255,
];
const W3: [u8; 8] = [0, 36, 72, 108, 147, 183, 219, 255];

#[test]
fn every_mode_is_listed_as_supported() {
    assert_eq!(BC7_SUPPORTED_MODES, [0, 1, 2, 3, 4, 5, 6, 7]);
}

#[test]
fn mode_is_the_lowest_set_bit_of_byte_zero() {
    for mode in 0..8u8 {
        let mut block = [0xffu8; 16];
        block[0] = 1 << mode;
        assert_eq!(bc7_mode(&block), Some(mode));
        // Higher bits of byte 0 belong to later fields, not the mode.
        block[0] = 0xff << mode;
        assert_eq!(bc7_mode(&block), Some(mode));
    }
}

#[test]
fn zero_low_byte_is_invalid_and_decodes_to_zero() {
    // Section 20.1: a low byte of 0 is reserved; decoders return 0 for all
    // channels of all texels, whatever the other 15 bytes hold.
    for fill in [0x00u8, 0xff, 0x5a] {
        let mut block = [fill; 16];
        block[0] = 0;
        assert_eq!(bc7_mode(&block), None);
        assert_eq!(decode_bc7(block), [[0; 4]; 16]);
    }
}

#[test]
fn mode6_four_bit_indices_anchor_and_endpoint_pbits() {
    // Mode 6: 7-bit RGBA endpoints, one P-bit per endpoint, 4-bit indices
    // (3 bits for the anchor texel 0). e0 = 0 with P = 0 gives 0;
    // e1 = 127 with P = 1 gives 0b1111_1111 = 255.
    let indices: [u32; 16] = core::array::from_fn(|t| t as u32);
    let block = |p1: u32| {
        let mut b = Block::new(6);
        for _ in 0..4 {
            b.put(0, 7).put(127, 7); // R, G, B, A for e0 and e1
        }
        b.put(0, 1).put(p1, 1);
        b.put(indices[0], 3);
        for &i in &indices[1..] {
            b.put(i, 4);
        }
        b.finish()
    };
    let out = decode_bc7(block(1));
    for t in 0..16 {
        assert_eq!(out[t], [W4[t]; 4], "texel {t}");
    }
    // With P = 0, e1 = 0b1111_1110 = 254: the P-bit is the lowest bit and no
    // bit is replicated into an 8-bit value.
    let out = decode_bc7(block(0));
    assert_eq!(out[15], [254; 4]);
    assert_eq!(out[0], [0; 4]);
}

#[test]
fn mode1_partition_shared_pbits_and_anchor() {
    // Mode 1, partition 0 (Table 114: columns 2 and 3 are subset 1; Table 118
    // anchor 15). 6-bit endpoints, one shared P-bit per subset, 3-bit indices,
    // 2 bits for texels 0 and 15. Subset 0 (P = 0): 0 -> 0, 63 -> 0b1111110
    // -> 252 | 1 = 253. Subset 1 (P = 1): 0 -> 0b0000001 -> 2, 63 -> 255.
    // Equation 2 for subset 0: 0, 36, 71, 107, 146, 182, 217, 253;
    // for subset 1: 2, 38, 73, 109, 148, 184, 219, 255.
    let idx: [u32; 16] = [0, 1, 2, 3, 4, 5, 6, 7, 0, 1, 2, 3, 4, 5, 6, 3];
    let mut b = Block::new(1);
    b.put(0, 6);
    for _ in 0..3 {
        b.put(0, 6).put(63, 6).put(0, 6).put(63, 6);
    }
    b.put(0, 1).put(1, 1);
    for (t, &i) in idx.iter().enumerate() {
        b.put(i, if t == 0 || t == 15 { 2 } else { 3 });
    }
    let expected: [u8; 16] = [
        0, 36, 73, 109, 146, 182, 219, 255, 0, 36, 73, 109, 146, 182, 219, 109,
    ];
    let out = decode_bc7(b.finish());
    for t in 0..16 {
        let v = expected[t];
        assert_eq!(out[t], [v, v, v, 255], "texel {t}");
    }
}

#[test]
fn mode4_rotation_and_index_selection() {
    // Mode 4: 5-bit colour, 6-bit alpha, 2-bit primary and 3-bit secondary
    // indices. e0 = (0, 0, 31, 0) -> (0, 0, 255, 0); e1 = (31, 0, 31, 63) ->
    // (255, 0, 255, 255). Primary indices are all 1 (weight 21 -> 84);
    // secondary index of texel t is t % 8 (texel 0 is the anchor).
    let block = |rotation: u32, selection: u32| {
        let mut b = Block::new(4);
        b.put(rotation, 2).put(selection, 1);
        b.put(0, 5).put(31, 5); // R
        b.put(0, 5).put(0, 5); // G
        b.put(31, 5).put(31, 5); // B
        b.put(0, 6).put(63, 6); // A
        b.put(1, 1);
        for _ in 1..16 {
            b.put(1, 2);
        }
        b.put(0, 2);
        for t in 1..16 {
            b.put(t % 8, 3);
        }
        b.finish()
    };
    for t in 0..16 {
        let s = W3[t % 8];
        // Selection 1: colour from the 3-bit indices, alpha from the 2-bit
        // ones, so RGBA = (s, 0, 255, 84); rotation 1 swaps A and R.
        assert_eq!(decode_bc7(block(1, 1))[t], [84, 0, 255, s], "texel {t}");
        // Selection 0: colour from the 2-bit indices (84, 0, 255), alpha s.
        assert_eq!(decode_bc7(block(0, 0))[t], [84, 0, 255, s], "texel {t}");
        assert_eq!(decode_bc7(block(2, 0))[t], [84, s, 255, 0], "texel {t}");
        assert_eq!(decode_bc7(block(3, 0))[t], [84, 0, s, 255], "texel {t}");
    }
}

#[test]
fn mode2_worked_example_index_bit_position() {
    // Section 20.1 example: mode 2, partition 6, texel (1, 2) = index 9 is in
    // subset 1 (endpoints 2 and 3) and its two index bits are block bits 115
    // and 116. Endpoint 3 is all ones (5 bits -> 255), every other endpoint 0.
    let mut b = Block::new(2);
    b.put(6, 6);
    for _ in 0..3 {
        b.put(0, 5)
            .put(0, 5)
            .put(0, 5)
            .put(31, 5)
            .put(0, 5)
            .put(0, 5);
    }
    b.put(0, 29);
    let mut block = b.finish();
    let mut bits = u128::from_le_bytes(block);
    bits |= 0b11 << 115;
    block = bits.to_le_bytes();
    let out = decode_bc7(block);
    for (t, px) in out.iter().enumerate() {
        let want = if t == 9 {
            [255, 255, 255, 255]
        } else {
            [0, 0, 0, 255]
        };
        assert_eq!(*px, want, "texel {t}");
    }
}

#[test]
fn mode0_three_subset_anchors_store_one_bit_less() {
    // Mode 0, partition 1: Table 116 anchor 3 and Table 117 anchor 8, plus
    // texel 0. Endpoints 4 bits + P-bit: e0 = 0 (P 0) -> 0, e1 = 15 (P 1) ->
    // 0b11111 -> 255. All 45 index bits set: anchors read 2 bits (index 3,
    // weight 27 -> 108), the rest 3 bits (index 7 -> 255).
    let mut b = Block::new(0);
    b.put(1, 4);
    for _ in 0..3 {
        for _ in 0..3 {
            b.put(0, 4).put(15, 4);
        }
    }
    for _ in 0..3 {
        b.put(0, 1).put(1, 1);
    }
    b.put(u32::MAX, 32).put((1 << 13) - 1, 13);
    let out = decode_bc7(b.finish());
    for (t, px) in out.iter().enumerate() {
        let v = if [0, 3, 8].contains(&t) { 108 } else { 255 };
        assert_eq!(*px, [v, v, v, 255], "texel {t}");
    }
}

#[test]
fn mode7_alpha_endpoints_with_pbits() {
    // Mode 7, partition 0: 5-bit RGBA + per-endpoint P-bit = 6 bits.
    // Subset 0: 0 (P 0) -> 0, 31 (P 0) -> 0b111110 -> 251.
    // Subset 1: 0 (P 1) -> 0b000001 -> 4, 31 (P 1) -> 255.
    // Indices all 3, except anchors 0 and 15 (1 bit, set to 1, weight 21):
    // subset 0 -> (21 * 251 + 32) >> 6 = 82; subset 1 -> (43 * 4 + 21 * 255 +
    // 32) >> 6 = 86.
    let mut b = Block::new(7);
    b.put(0, 6);
    for _ in 0..4 {
        b.put(0, 5).put(31, 5).put(0, 5).put(31, 5);
    }
    b.put(0, 1).put(0, 1).put(1, 1).put(1, 1);
    for t in 0..16 {
        if t == 0 || t == 15 {
            b.put(1, 1);
        } else {
            b.put(3, 2);
        }
    }
    let out = decode_bc7(b.finish());
    for (t, px) in out.iter().enumerate() {
        let v = match t {
            0 => 82,
            15 => 86,
            _ if t % 4 < 2 => 251,
            _ => 255,
        };
        assert_eq!(*px, [v; 4], "texel {t}");
    }
}

#[test]
fn mode3_eight_bit_endpoints_need_no_replication() {
    // Mode 3: 7 bits + P-bit = 8 bits. Subset 0 e0 = 85 (P 1) -> 171,
    // subset 1 e0 = 42 (P 0) -> 84; all indices 0. Alpha is 255.
    let mut b = Block::new(3);
    b.put(0, 6);
    for _ in 0..3 {
        b.put(85, 7).put(0, 7).put(42, 7).put(0, 7);
    }
    b.put(1, 1).put(0, 1).put(0, 1).put(0, 1);
    b.put(0, 30);
    let out = decode_bc7(b.finish());
    for (t, px) in out.iter().enumerate() {
        let v = if t % 4 < 2 { 171 } else { 84 };
        assert_eq!(*px, [v, v, v, 255], "texel {t}");
    }
}

#[test]
fn mode5_rotation_with_eight_bit_alpha() {
    // Mode 5: 7-bit colour (-> 8 bits by replication), 8-bit alpha, 2-bit
    // colour and alpha indices. e0 = (127, 0, 0, 10), e1 = (127, 0, 0, 200);
    // 127 -> 255. Colour indices 0; alpha indices 3 (anchor texel 0 has 1
    // bit, set to 1: (43 * 10 + 21 * 200 + 32) >> 6 = 72). Rotation 3 swaps
    // A and B.
    let mut b = Block::new(5);
    b.put(3, 2);
    b.put(127, 7)
        .put(127, 7)
        .put(0, 7)
        .put(0, 7)
        .put(0, 7)
        .put(0, 7);
    b.put(10, 8).put(200, 8);
    b.put(0, 31);
    b.put(1, 1);
    for _ in 1..16 {
        b.put(3, 2);
    }
    let out = decode_bc7(b.finish());
    assert_eq!(out[0], [255, 0, 72, 0]);
    for (t, px) in out.iter().enumerate().skip(1) {
        assert_eq!(*px, [255, 0, 200, 0], "texel {t}");
    }
}

#[test]
fn all_one_payloads_saturate() {
    // Every field all ones: maximum endpoints expand to 255 and every index
    // selects e1 (weight 64), so every channel is 255 in every mode.
    for mode in 0..8u32 {
        // Modes 4 and 5 also get rotation 3 (swap A and B, both 255).
        let bits = u128::MAX << mode;
        let out = decode_bc7(bits.to_le_bytes());
        assert_eq!(out, [[255; 4]; 16], "mode {mode}");
    }
}

/// Known-answer vectors from an independent clean-room reference decoder.
#[test]
fn independent_known_answer_vectors() {
    let text = include_str!("fixtures/bc7_vectors.txt");
    let mut count = 0;
    for line in text
        .lines()
        .filter(|l| !l.starts_with('#') && !l.is_empty())
    {
        let mut fields = line.split_whitespace();
        let tag = fields.next().unwrap();
        let hex = fields.next().unwrap();
        let block: [u8; 16] =
            core::array::from_fn(|i| u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).unwrap());
        let want: Vec<[u8; 4]> = fields
            .map(|t| {
                let v = u32::from_str_radix(t, 16).unwrap();
                v.to_be_bytes()
            })
            .collect();
        assert_eq!(want.len(), 16, "{tag}");
        assert_eq!(decode_bc7(block).to_vec(), want, "{tag} {hex}");
        count += 1;
    }
    assert!(count >= 300, "only {count} vectors");
}
