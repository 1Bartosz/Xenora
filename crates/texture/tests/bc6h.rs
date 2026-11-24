// SPDX-License-Identifier: GPL-3.0-or-later
//! BC6H subset tests.
//!
//! Hand-worked expectations follow the Khronos Data Format Specification 1.3,
//! section 20.2 (Tables 121 and 122, unquantization and final unquantization)
//! and the BC7 weight table (Table 119). The known-answer file holds vectors
//! produced by a separate clean-room reference decoder, not by this crate.

use xenora_texture::{
    BC6H_SUPPORTED_MODES, Bc6hError, bc6h_mode, decode_bc6h_sfloat, decode_bc6h_ufloat,
};

/// Packs fields into a block from bit 0 upward, as Table 122 reads them.
#[derive(Default)]
struct Writer {
    bits: u128,
    offset: u32,
}

impl Writer {
    fn put(&mut self, value: u64, count: u32) -> &mut Self {
        self.bits |= u128::from(value & ((1 << count) - 1)) << self.offset;
        self.offset += count;
        self
    }

    /// Table 122's reversed field `v[low..high]`: the first block bit holds
    /// bit `high` of `value`, the next `high - 1`, down to `low`.
    fn reversed(&mut self, value: u64, low: u32, high: u32) -> &mut Self {
        for bit in (low..=high).rev() {
            self.put(value >> bit, 1);
        }
        self
    }

    /// Writes the 63 index bits: texel 0 is the anchor (3 bits, implicit top
    /// bit 0), every other texel 4 bits.
    fn indices(&mut self, indices: [u64; 16]) -> [u8; 16] {
        assert_eq!(self.offset, 65);
        assert!(indices[0] < 8);
        self.put(indices[0], 3);
        for &index in &indices[1..] {
            self.put(index, 4);
        }
        assert_eq!(self.offset, 128);
        self.bits.to_le_bytes()
    }
}

/// Index t for texel t, except the anchor texel 0, which must be below 8.
const RAMP: [u64; 16] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15];

/// Mode 3 (Microsoft mode 11): 10-bit endpoints stored directly.
fn mode3(e0: [u64; 3], e1: [u64; 3], indices: [u64; 16]) -> [u8; 16] {
    let mut w = Writer::default();
    w.put(3, 5);
    for value in e0.into_iter().chain(e1) {
        w.put(value, 10);
    }
    w.indices(indices)
}

#[test]
fn mode3_unsigned_spans_zero_to_the_largest_half() {
    let block = mode3([0; 3], [1023; 3], RAMP);
    assert_eq!(bc6h_mode(block), 3);
    let texels = decode_bc6h_ufloat(block).unwrap();
    // Endpoint 0 unquantizes to 0 and the all-ones endpoint to 0xffff, which
    // the final (i * 31) >> 6 maps to 0x7bff, 65504.0.
    assert_eq!(texels[0], [0; 3]);
    assert_eq!(texels[15], [0x7bff; 3]);
    // Index 8 has weight 34: i = (34 * 0xffff + 32) >> 6 = 34815, and
    // (34815 * 31) >> 6 = 16863 = 0x41df.
    assert_eq!(texels[8], [0x41df; 3]);
    // Index 1 has weight 4: i = (4 * 0xffff + 32) >> 6 = 4096, then
    // (4096 * 31) >> 6 = 1984 = 0x07c0.
    assert_eq!(texels[1], [0x07c0; 3]);
}

#[test]
fn mode3_signed_sign_extends_and_saturates_both_ends() {
    // 0x201 is -511 in 10 bits; |x| >= 2^9 - 1 unquantizes to -0x7fff, and
    // the final step gives ((0x7fff * 31) >> 5) | 0x8000 = 0xfbff (-65504.0).
    // 511 gives 0x7fff -> 0x7bff (65504.0).
    let block = mode3([0x201, 0x201, 0], [511, 0, 0x201], RAMP);
    let texels = decode_bc6h_sfloat(block).unwrap();
    assert_eq!(texels[0], [0xfbff, 0xfbff, 0x0000]);
    assert_eq!(texels[15], [0x7bff, 0x0000, 0xfbff]);
    // Index 8 between -0x7fff and 0x7fff: i = (30 * -32767 + 34 * 32767 + 32)
    // >> 6 = 2048, so (2048 * 31) >> 5 = 1984 = 0x07c0 in red.
    assert_eq!(texels[8][0], 0x07c0);
    // Unsigned data reads 0x201 as 513: ((513 << 15) + 0x4000) >> 9 = 32864,
    // and (32864 * 31) >> 6 = 15918 = 0x3e2e.
    assert_eq!(decode_bc6h_ufloat(block).unwrap()[0][0], 0x3e2e);
}

#[test]
fn mode7_wraps_the_transformed_endpoint_at_eleven_bits() {
    // Mode 7 (Microsoft 12): R0[9:0], G0[9:0], B0[9:0], then per channel
    // R1[8:0] and R0[10]. E0 = 2047 in every channel, delta +1, so
    // E1 = (2047 + 1) & 0x7ff = 0.
    let mut w = Writer::default();
    w.put(7, 5);
    for _ in 0..3 {
        w.put(0x3ff, 10);
    }
    for _ in 0..3 {
        w.put(1, 9).put(1, 1);
    }
    let block = w.indices(RAMP);
    let texels = decode_bc6h_ufloat(block).unwrap();
    assert_eq!(texels[0], [0x7bff; 3]);
    assert_eq!(texels[15], [0; 3]);
    // Signed: E0 = 2047 is -1 in 11 bits and the delta +1 makes E1 = 0.
    // -1 unquantizes to -((1 << 15) + 0x4000) >> 10 = -48, final -(48 * 31 >> 5)
    // = -46 -> 0x802e.
    let texels = decode_bc6h_sfloat(block).unwrap();
    assert_eq!(texels[0], [0x802e; 3]);
    assert_eq!(texels[15], [0; 3]);
}

#[test]
fn mode15_reads_its_reversed_high_bits() {
    // Mode 15 (Microsoft 14): E0[9:0] per channel, then per channel E1[3:0]
    // and E0[10..15] reversed. E0 = 0x8000 has only bit 15 set, which is
    // the first bit of the reversed field. 16-bit endpoints are not
    // unquantized, so E0 reads (0x8000 * 31) >> 6 = 0x3e00 unsigned.
    let mut w = Writer::default();
    w.put(15, 5);
    for _ in 0..3 {
        w.put(0, 10);
    }
    for _ in 0..3 {
        w.put(1, 4).reversed(0x8000, 10, 15);
    }
    let block = w.indices(RAMP);
    let texels = decode_bc6h_ufloat(block).unwrap();
    assert_eq!(texels[0], [0x3e00; 3]);
    // E1 = 0x8001: (0x8001 * 31) >> 6 = 0x3e00 as well.
    assert_eq!(texels[15], [0x3e00; 3]);
    // Swapping the reversed order would place bit 15 at bit 10: E0 = 0x400,
    // which would read (0x400 * 31) >> 6 = 0x1f0 instead.
    assert_ne!(texels[0][0], 0x01f0);
    // Signed: 0x8000 is -32768 and passes through unquantization; the final
    // step gives ((32768 * 31) >> 5) | 0x8000 = 0xfc00, negative infinity.
    assert_eq!(decode_bc6h_sfloat(block).unwrap()[0], [0xfc00; 3]);
}

#[test]
fn mode11_places_its_two_reversed_bits() {
    // Mode 11 (Microsoft 13): E1[7:0] then E0[10..11] reversed. E0 = 0x800
    // (bit 11 only) and delta 0; 12-bit 0x800 = 2048 unquantizes to
    // ((2048 << 15) + 0x4000) >> 11 = 32776, final (32776 * 31) >> 6 = 15875.
    let mut w = Writer::default();
    w.put(11, 5);
    for _ in 0..3 {
        w.put(0, 10);
    }
    for _ in 0..3 {
        w.put(0, 8).reversed(0x800, 10, 11);
    }
    let block = w.indices([0; 16]);
    assert_eq!(decode_bc6h_ufloat(block).unwrap()[5], [15875; 3]);
}

#[test]
fn reserved_modes_decode_to_zero() {
    for mode in [19, 23, 27, 31] {
        let block = [
            mode, 0xff, 0x12, 0x34, 0x56, 0x78, 0x9a, 0xbc, 0xde, 0xf0, 1, 2, 3, 4, 5, 6,
        ];
        assert_eq!(bc6h_mode(block), mode);
        assert_eq!(decode_bc6h_ufloat(block).unwrap(), [[0; 3]; 16]);
        assert_eq!(decode_bc6h_sfloat(block).unwrap(), [[0; 3]; 16]);
    }
}

#[test]
fn rejects_every_two_subset_mode() {
    let two_subset = [0, 1, 2, 6, 10, 14, 18, 22, 26, 30];
    for mode in 0..32u8 {
        let block = [mode, 0xaa, 0x55, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        let number = bc6h_mode(block);
        if two_subset.contains(&number) {
            assert!(!BC6H_SUPPORTED_MODES.contains(&number));
            assert_eq!(
                decode_bc6h_ufloat(block),
                Err(Bc6hError::UnsupportedMode(number))
            );
            assert_eq!(
                decode_bc6h_sfloat(block),
                Err(Bc6hError::UnsupportedMode(number))
            );
        }
    }
    // Two-bit modes: the low bits 0 and 1 select modes 0 and 1 whatever the
    // next three bits hold.
    assert_eq!(
        bc6h_mode([0b11100, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]),
        0
    );
    assert_eq!(
        bc6h_mode([0b11101, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]),
        1
    );
}

fn hex16(text: &str) -> u16 {
    u16::from_str_radix(text, 16).unwrap()
}

#[test]
fn matches_independent_known_answers_by_mode() {
    let text = include_str!("fixtures/bc6h_vectors.txt");
    let mut per_mode = std::collections::BTreeMap::<u8, usize>::new();
    for line in text.lines().filter(|line| !line.starts_with('#')) {
        let fields: Vec<_> = line.split(' ').collect();
        let hex = fields[2];
        let block: [u8; 16] =
            std::array::from_fn(|i| u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).unwrap());
        let want: Vec<[u16; 3]> = fields[3..]
            .iter()
            .map(|t| [0, 1, 2].map(|c| hex16(&t[4 * c..4 * c + 4])))
            .collect();
        let got = if fields[1] == "1" {
            decode_bc6h_sfloat(block)
        } else {
            decode_bc6h_ufloat(block)
        }
        .unwrap();
        assert_eq!(got[..], want[..], "vector {line}");
        *per_mode.entry(bc6h_mode(block)).or_default() += 1;
    }
    // 16 vectors (8 unsigned, 8 signed) for each supported and reserved mode.
    let expected: Vec<_> = [3, 7, 11, 15, 19, 23, 27, 31].map(|m| (m, 16)).into();
    assert_eq!(per_mode.into_iter().collect::<Vec<_>>(), expected);
}
