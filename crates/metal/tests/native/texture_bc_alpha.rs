// SPDX-License-Identifier: GPL-3.0-or-later
//! Native Metal upload and readback of original BC2 and BC3 blocks.
//!
//! Every block below was written for this test. The full-texel reference is
//! the `xenora-texture` CPU decoder; the anchor texels are worked by hand from
//! the Microsoft Direct3D 11 block-compression documentation (BC2, BC3) and
//! the Khronos Data Format Specification 1.3 (S3TC DXT3/DXT5). Apple's GPU
//! decoder is the value under test, so expected and actual values come from
//! different decoders.

use super::{context, native};
use xenora_texture::{BcAlphaFormat, BcAlphaUpload, BcAlphaUploadDesc, decode_bc2, decode_bc3};

/// The specifications define colour and BC3 alpha interpolation in real
/// numbers, and the CPU reference truncates them. On an Apple M5 Pro the
/// native decoder's interpolated values lie within 1.14 codes of the real
/// value (they match 6-bit fixed-point weights), so they can sit two codes
/// above the truncated reference: 124 against 122 for BC3 alpha
/// (4 * 200 + 3 * 20) / 7 = 122.86. Endpoints, BC2 alpha and BC3's fixed
/// 0 and 255 are exact and are pinned by the anchor checks.
const TOLERANCE: u8 = 2;

/// Bound for anchors against the real-number value of an interpolated code.
const REAL_TOLERANCE: f64 = 1.5;

fn assert_near(label: &str, actual: u8, real: f64) {
    assert!(
        (f64::from(actual) - real).abs() <= REAL_TOLERANCE,
        "{label}: got {actual}, real value {real:.2}"
    );
}

const WIDTH: usize = 5;
const HEIGHT: usize = 5;
const ROW_PITCH: usize = 48;
const SLICE_PITCH: usize = 112;

/// Colour half: two little-endian RGB565 endpoints, then 2-bit indices.
/// Every row of 0xe4 selects palette entries 0, 1, 2, 3 from left to right.
fn colour((first, second): (u16, u16)) -> [u8; 8] {
    let [a, b] = first.to_le_bytes();
    let [c, d] = second.to_le_bytes();
    [a, b, c, d, 0xe4, 0xe4, 0xe4, 0xe4]
}

/// BC3 alpha half whose texel `t` selects palette entry `(t + 6) % 8`, so
/// every row starts with the indices 6 and 7 and a full block uses all eight.
fn bc3_alpha(first: u8, second: u8) -> [u8; 8] {
    let bits = (0..16u64).fold(0, |bits, texel| bits | (((texel + 6) % 8) << (3 * texel)));
    let i = bits.to_le_bytes();
    [first, second, i[0], i[1], i[2], i[3], i[4], i[5]]
}

fn block(alpha: [u8; 8], colour: [u8; 8]) -> [u8; 16] {
    let mut block = [0; 16];
    block[..8].copy_from_slice(&alpha);
    block[8..].copy_from_slice(&colour);
    block
}

// First endpoint greater: red over blue.
const DESCENDING: (u16, u16) = (0xf800, 0x001f);
// First endpoint smaller, with odd channel values that exercise rounding.
const ASCENDING: (u16, u16) = (0x52aa, 0xad55);
// Equal endpoints: BC1 would decode index 3 as transparent black.
const EQUAL: (u16, u16) = (0x07e0, 0x07e0);

/// White with zero alpha (BC2 nibbles 0; BC3 endpoints 0 and 0) in every
/// padding slot, so a wrong row or slice pitch reads visibly wrong texels.
const PAD: [u8; 16] = [0, 0, 0, 0, 0, 0, 0, 0, 0xff, 0xff, 0xff, 0xff, 0, 0, 0, 0];

/// Logical blocks of one layer: the 2×2 base-level grid, then mip 1.
type Layer = [[u8; 16]; 5];

/// Two layers of a 5×5 texture with two mips. Each subresource has seven
/// physical blocks: rows of two logical blocks plus one padding block, a
/// padding block that closes the slice, and mip 1 uses only its block zero.
fn upload_bytes(layers: [Layer; 2]) -> Vec<u8> {
    let mut bytes = Vec::new();
    for [b0, b1, b2, b3, small] in layers {
        for slot in [b0, b1, PAD, b2, b3, PAD, PAD] {
            bytes.extend_from_slice(&slot);
        }
        for slot in [small, PAD, PAD, PAD, PAD, PAD, PAD] {
            bytes.extend_from_slice(&slot);
        }
    }
    assert_eq!(bytes.len(), 2 * 2 * SLICE_PITCH);
    bytes
}

/// Crops decoded blocks (a row-major block grid) to a `width`×`height` level
/// of words with red in the lowest byte, as the readback kernel writes them.
fn reference(blocks: &[[[u8; 4]; 16]], blocks_x: usize, width: usize, height: usize) -> Vec<u32> {
    let mut texels = Vec::new();
    for y in 0..height {
        for x in 0..width {
            let decoded = blocks[(y / 4) * blocks_x + x / 4];
            texels.push(u32::from_le_bytes(decoded[(y % 4) * 4 + x % 4]));
        }
    }
    texels
}

fn alpha(word: u32) -> u8 {
    word.to_le_bytes()[3]
}

/// Returns the largest per-channel difference, failing on any texel above the
/// tolerance with its index and both values.
fn assert_close(label: &str, actual: &[u32], expected: &[u32]) -> u8 {
    assert_eq!(actual.len(), expected.len(), "{label}: texel count");
    let mut worst = 0;
    for (texel, (&got, &want)) in actual.iter().zip(expected).enumerate() {
        let diff = got
            .to_le_bytes()
            .iter()
            .zip(want.to_le_bytes())
            .map(|(g, w)| g.abs_diff(w))
            .max()
            .unwrap();
        assert!(
            diff <= TOLERANCE,
            "{label}: texel {texel} got {got:#010x}, want {want:#010x}"
        );
        worst = worst.max(diff);
    }
    worst
}

/// GPU reads of both layers: `[layer][mip]`.
type Reads = [[Vec<u32>; 2]; 2];

/// Uploads both layers, reads all four subresources on the GPU, compares
/// them with the CPU decoder and returns the reads with the worst difference.
fn read_and_compare(
    format: BcAlphaFormat,
    decode: fn([u8; 16]) -> [[u8; 4]; 16],
    layers: [Layer; 2],
) -> (Reads, u8) {
    let bytes = upload_bytes(layers);
    let upload = BcAlphaUpload::new(
        &bytes,
        BcAlphaUploadDesc {
            width: WIDTH,
            height: HEIGHT,
            mips: 2,
            layers: 2,
            format,
            row_pitch: ROW_PITCH,
            slice_pitch: SLICE_PITCH,
        },
    )
    .unwrap();
    let gpu = context();
    let mut worst = 0;
    let reads = std::array::from_fn(|layer| {
        let decoded = layers[layer].map(decode);
        let base = native(gpu.read_bc_alpha_upload(&upload, layer, 0));
        let expected = reference(&decoded[..4], 2, WIDTH, HEIGHT);
        let label = format!("{format:?} layer {layer}");
        worst = worst.max(assert_close(&format!("{label} mip 0"), &base, &expected));
        let small = native(gpu.read_bc_alpha_upload(&upload, layer, 1));
        let expected = reference(&decoded[4..], 1, 2, 2);
        worst = worst.max(assert_close(&format!("{label} mip 1"), &small, &expected));
        [base, small]
    });
    for (layer, mip) in [(2, 0), (0, 2)] {
        assert_eq!(
            gpu.read_bc_alpha_upload(&upload, layer, mip)
                .unwrap_err()
                .operation,
            "texture subresource"
        );
    }
    (reads, worst)
}

#[test]
fn reads_pitched_bc2_explicit_alpha_across_mips_and_layers() {
    // Nibble t in texel t, low nibble first: alpha t * 17.
    let ramp = [0x10, 0x32, 0x54, 0x76, 0x98, 0xba, 0xdc, 0xfe];
    let reverse = ramp.map(|byte: u8| !byte);
    let opaque = [0xff; 8];
    let [descending, ascending, equal] = [DESCENDING, ASCENDING, EQUAL].map(colour);
    let (reads, worst) = read_and_compare(
        BcAlphaFormat::Bc2,
        decode_bc2,
        [
            [
                block(ramp, descending),
                block(reverse, ascending),
                block(opaque, equal),
                block(ramp, ascending),
                block(reverse, descending),
            ],
            [
                block(opaque, ascending),
                block(ramp, equal),
                block(reverse, descending),
                block(opaque, descending),
                block(ramp, equal),
            ],
        ],
    );

    // Hand-derived anchors, independent of the CPU decoder.
    let base = &reads[0][0];
    // (0, 0): red endpoint, nibble 0. (1, 0): blue endpoint, nibble 1.
    assert_eq!(base[0], 0x0000_00ff);
    assert_eq!(base[1], 0x11ff_0000);
    // (0, 2) is texel 8 of block 0: red endpoint, nibble 8 -> 0x88.
    assert_eq!(base[2 * WIDTH], 0x8800_00ff);
    // (3, 3) is texel 15 of block 0: nibble 15 -> opaque.
    assert_eq!(alpha(base[3 * WIDTH + 3]), 0xff);
    // (4, 0) is texel 0 of block 1: reversed ramp, nibble 15.
    assert_eq!(alpha(base[4]), 0xff);
    // (3, 4) is index 3 of block 2's equal green endpoints: opaque green,
    // not BC1's transparent black.
    assert_eq!(base[4 * WIDTH + 3], 0xff00_ff00);
    // Layer 1 mip 1: green endpoints, ramp nibbles 0, 1, 4, 5.
    assert_eq!(
        reads[1][1],
        [0x0000_ff00, 0x1100_ff00, 0x4400_ff00, 0x5500_ff00]
    );
    println!("checked 58 BC2 texels from four subresources, worst channel difference {worst}");
}

#[test]
fn reads_pitched_bc3_interpolated_alpha_across_mips_and_layers() {
    // First endpoint greater: eight-value palette.
    let eight = bc3_alpha(200, 20);
    // First endpoint smaller: six-value palette with fixed 0 and 255.
    let six = bc3_alpha(20, 200);
    // Equal endpoints also select the six-value palette.
    let level = bc3_alpha(128, 128);
    let [descending, ascending, equal] = [DESCENDING, ASCENDING, EQUAL].map(colour);
    let (reads, worst) = read_and_compare(
        BcAlphaFormat::Bc3,
        decode_bc3,
        [
            [
                block(eight, descending),
                block(six, ascending),
                block(level, equal),
                block(eight, ascending),
                block(six, descending),
            ],
            [
                block(six, equal),
                block(eight, ascending),
                block(level, descending),
                block(six, ascending),
                block(eight, equal),
            ],
        ],
    );

    // Hand-derived anchors, independent of the CPU decoder.
    let base = &reads[0][0];
    let a = |x: usize, y: usize| alpha(base[y * WIDTH + x]);
    // Block 0, eight-value: row 0 holds indices 6, 7, 0, 1, and row 1
    // indices 2, 3, 4, 5, each (k = index - 1) ((7 - k) * 200 + k * 20) / 7.
    assert_near("index 6", a(0, 0), (2.0 * 200.0 + 5.0 * 20.0) / 7.0);
    assert_near("index 7", a(1, 0), (200.0 + 6.0 * 20.0) / 7.0);
    for (x, k) in (0..4).zip(1..) {
        let real = f64::from((7 - k) * 200 + k * 20) / 7.0;
        assert_near(&format!("index {}", k + 1), a(x, 1), real);
    }
    // Then the endpoints themselves, exactly; texel (0, 0) is the red endpoint.
    assert_eq!([a(2, 0), a(3, 0)], [200, 20]);
    assert_eq!(base[0] & 0x00ff_ffff, 0x0000_00ff);
    // Block 1, six-value: column 4 holds indices 6, 2, 6, 2. Index 6 is the
    // fixed 0 and index 2 is (4 * 20 + 200) / 5 = 56.
    assert_eq!([a(4, 0), a(4, 2)], [0, 0]);
    assert_near("six-value index 2", a(4, 1), 56.0);
    assert_near("six-value index 2", a(4, 3), 56.0);
    // Block 2, equal endpoints of 128: row 4 holds indices 6, 7, 0, 1, so the
    // six-value mode shows as the fixed 0 and 255.
    assert_eq!([a(0, 4), a(1, 4), a(2, 4), a(3, 4)], [0, 255, 128, 128]);
    // Mip 1, six-value: indices 6, 7, 2, 3 -> 0, 255, 56, (3 * 20 + 2 * 200) / 5 = 92.
    let small: Vec<_> = reads[0][1].iter().copied().map(alpha).collect();
    assert_eq!(small[..2], [0, 255]);
    assert_near("six-value index 2", small[2], 56.0);
    assert_near("six-value index 3", small[3], 92.0);
    println!("checked 58 BC3 texels from four subresources, worst channel difference {worst}");
}

/// Reads one-mip, one-layer uploads of `width`×`height` from `blocks` laid
/// out with `row_pitch`, comparing with the CPU decoder.
fn read_edge(
    format: BcAlphaFormat,
    decode: fn([u8; 16]) -> [[u8; 4]; 16],
    blocks: &[[u8; 16]],
    (width, height, row_pitch): (usize, usize, usize),
) -> Vec<u32> {
    let bytes = blocks.as_flattened();
    let upload = BcAlphaUpload::new(
        bytes,
        BcAlphaUploadDesc {
            width,
            height,
            mips: 1,
            layers: 1,
            format,
            row_pitch,
            slice_pitch: bytes.len(),
        },
    )
    .unwrap();
    let actual = native(context().read_bc_alpha_upload(&upload, 0, 0));
    let decoded: Vec<_> = blocks.iter().copied().map(decode).collect();
    let expected = reference(&decoded, width.div_ceil(4), width, height);
    assert_close(&format!("{format:?} {width}x{height}"), &actual, &expected);
    actual
}

#[test]
fn reads_bc2_and_bc3_edge_blocks() {
    let ramp = [0x10, 0x32, 0x54, 0x76, 0x98, 0xba, 0xdc, 0xfe];
    let reverse = ramp.map(|byte: u8| !byte);
    let [descending, equal] = [DESCENDING, EQUAL].map(colour);
    // 5×3 is one row of two blocks: Metal receives a zero row stride, and
    // both blocks are cut at row 3, the second also at column 1. The row
    // pitch keeps one padding block.
    let wide = (5, 3, 48);
    let bc2 = read_edge(
        BcAlphaFormat::Bc2,
        decode_bc2,
        &[block(ramp, descending), block(reverse, equal), PAD],
        wide,
    );
    // (0, 0) red, nibble 0; (4, 0) green, reversed nibble 15; (0, 2) red,
    // nibble 8; (4, 2) green, reversed nibble 7.
    assert_eq!(
        [bc2[0], bc2[4], bc2[10], bc2[14]],
        [0x0000_00ff, 0xff00_ff00, 0x8800_00ff, 0x7700_ff00]
    );
    let bc3 = read_edge(
        BcAlphaFormat::Bc3,
        decode_bc3,
        &[
            block(bc3_alpha(200, 20), descending),
            block(bc3_alpha(20, 200), equal),
            PAD,
        ],
        wide,
    );
    // Row 0 of block 0 is indices 6, 7, 0, 1: (2, 0) and (3, 0) are the
    // endpoints 200 and 20. Column 0 of block 1 is indices 6, 2, 6: green
    // with the fixed 0, then (4 * 20 + 200) / 5 = 56.
    assert_eq!([alpha(bc3[2]), alpha(bc3[3])], [200, 20]);
    assert_eq!([bc3[4], bc3[14]], [0x0000_ff00; 2]);
    assert_near("six-value index 2", alpha(bc3[9]), 56.0);
    // 1×1 is a single block cut to its first texel.
    let one = (1, 1, 16);
    let bc2 = read_edge(
        BcAlphaFormat::Bc2,
        decode_bc2,
        &[block(reverse, descending)],
        one,
    );
    assert_eq!(bc2, [0xff00_00ff]);
    let bc3 = read_edge(
        BcAlphaFormat::Bc3,
        decode_bc3,
        &[block(bc3_alpha(20, 200), descending)],
        one,
    );
    assert_eq!(bc3, [0x0000_00ff]);
    println!("checked 32 BC2 and BC3 texels from 5x3 and 1x1 edge levels");
}
