// SPDX-License-Identifier: GPL-3.0-or-later
//! Native Metal upload and readback of original BC4 and BC5 blocks.
//!
//! Every block below was written for this test. The full-texel reference is
//! the `xenora-texture` CPU decoder; the anchor texels are worked by hand from
//! the Microsoft Direct3D 11 block-compression documentation (BC4, BC5) and
//! the Khronos Data Format Specification 1.3 (RGTC). Apple's GPU decoder is
//! the value under test, so expected and actual values come from different
//! decoders.

use super::{context, native};
use xenora_texture::{
    BcChannelFormat, BcChannelUpload, BcChannelUploadDesc, decode_bc4_snorm, decode_bc4_unorm,
    decode_bc5_snorm, decode_bc5_unorm,
};

/// The specifications define interpolation in real numbers and the CPU
/// reference truncates toward zero. On an Apple M5 Pro every format's
/// interpolated codes sit up to two codes from that reference; each test
/// prints its measured worst case. Endpoints and the six-value fixed codes
/// are exact and are pinned by the anchor checks.
const TOLERANCE: u8 = 2;

/// Bound for anchors against the real-number value of an interpolated code.
/// The measured worst on an Apple M5 Pro is 1.71 codes: signed eight-value
/// index 4 of 100 and -100 is (4 * 100 - 3 * 100) / 7 = 14.29 and reads 16.
const REAL_TOLERANCE: f64 = 2.0;

const WIDTH: usize = 5;
const HEIGHT: usize = 5;

/// One 8-byte channel block whose texel `t` selects palette entry
/// `(t + 6) % 8`: every row starts with indices 6 and 7, and a full block
/// uses all eight.
fn channel(first: u8, second: u8) -> [u8; 8] {
    let bits = (0..16u64).fold(0, |bits, texel| bits | (((texel + 6) % 8) << (3 * texel)));
    let i = bits.to_le_bytes();
    [first, second, i[0], i[1], i[2], i[3], i[4], i[5]]
}

fn signed(first: i8, second: i8) -> [u8; 8] {
    channel(first.to_le_bytes()[0], second.to_le_bytes()[0])
}

/// Index 0 everywhere at endpoint 0x7f: 127 unsigned or 1.0 signed, so a
/// wrong row or slice pitch reads visibly wrong texels.
const PAD: [u8; 8] = [0x7f, 0x7f, 0, 0, 0, 0, 0, 0];

/// Logical blocks of one layer: the 2×2 base-level grid, then mip 1.
type Layer = [Vec<u8>; 5];

struct Case {
    format: BcChannelFormat,
    /// Expected little-endian texel words of one block.
    reference: fn(&[u8]) -> [u32; 16],
    layers: [Layer; 2],
}

fn unorm_word(r: u8, g: u8) -> u32 {
    u32::from_le_bytes([r, g, 0, 255])
}

/// Both -128 and -127 decode to -1.0, so the native output is -127 for each.
fn snorm_word(r: i8, g: i8) -> u32 {
    let byte = |value: i8| value.max(-127).to_le_bytes()[0];
    u32::from_le_bytes([byte(r), byte(g), 0, 127])
}

fn bc4_unorm(block: &[u8]) -> [u32; 16] {
    decode_bc4_unorm(block.try_into().unwrap()).map(|r| unorm_word(r, 0))
}

fn bc4_snorm(block: &[u8]) -> [u32; 16] {
    decode_bc4_snorm(block.try_into().unwrap()).map(|r| snorm_word(r, 0))
}

fn bc5_unorm(block: &[u8]) -> [u32; 16] {
    decode_bc5_unorm(block.try_into().unwrap()).map(|[r, g]| unorm_word(r, g))
}

fn bc5_snorm(block: &[u8]) -> [u32; 16] {
    decode_bc5_snorm(block.try_into().unwrap()).map(|[r, g]| snorm_word(r, g))
}

fn dual(red: [u8; 8], green: [u8; 8]) -> Vec<u8> {
    [red, green].concat()
}

/// Seven physical blocks per subresource: rows of two logical blocks plus
/// one padding block, a padding block closing the slice, and mip 1 using
/// only its block zero. Returns the bytes with the row and slice pitch.
fn upload_bytes(format: BcChannelFormat, layers: &[Layer; 2]) -> (Vec<u8>, usize, usize) {
    let block = format.block_bytes();
    let pad = PAD.repeat(block / PAD.len());
    let mut bytes = Vec::new();
    for [b0, b1, b2, b3, small] in layers {
        for slot in [b0, b1, &pad, b2, b3, &pad, &pad] {
            bytes.extend_from_slice(slot);
        }
        bytes.extend_from_slice(small);
        for _ in 0..6 {
            bytes.extend_from_slice(&pad);
        }
    }
    (bytes, 3 * block, 7 * block)
}

/// Crops decoded blocks (a row-major block grid) to a `width`×`height` level.
fn crop<T: Copy>(blocks: &[[T; 16]], blocks_x: usize, width: usize, height: usize) -> Vec<T> {
    let mut texels = Vec::new();
    for y in 0..height {
        for x in 0..width {
            texels.push(blocks[(y / 4) * blocks_x + x / 4][(y % 4) * 4 + x % 4]);
        }
    }
    texels
}

/// Per-channel distances; signed formats compare bytes as two's complement.
fn distances(format: BcChannelFormat, got: u32, want: u32) -> [u8; 4] {
    let signed = matches!(
        format,
        BcChannelFormat::Bc4Snorm | BcChannelFormat::Bc5Snorm
    );
    let (got, want) = (got.to_le_bytes(), want.to_le_bytes());
    std::array::from_fn(|c| {
        if signed {
            (got[c] as i8).abs_diff(want[c] as i8)
        } else {
            got[c].abs_diff(want[c])
        }
    })
}

/// Per-channel tolerance of each texel: zero where a channel block's index
/// selects a stored endpoint or a six-value fixed code (and for the constant
/// channels), [`TOLERANCE`] only for interpolated codes.
fn tolerances(format: BcChannelFormat, block: &[u8]) -> [[u8; 4]; 16] {
    let signed = matches!(
        format,
        BcChannelFormat::Bc4Snorm | BcChannelFormat::Bc5Snorm
    );
    let limit = |channel: &[u8], t: usize| {
        let mut bits = [0; 8];
        bits[..6].copy_from_slice(&channel[2..8]);
        let index = (u64::from_le_bytes(bits) >> (3 * t)) & 7;
        let six_value = if signed {
            channel[0] as i8 <= channel[1] as i8
        } else {
            channel[0] <= channel[1]
        };
        if index < 2 || (six_value && index >= 6) {
            0
        } else {
            TOLERANCE
        }
    };
    std::array::from_fn(|t| {
        let green = if block.len() == 16 {
            limit(&block[8..], t)
        } else {
            0
        };
        [limit(&block[..8], t), green, 0, 0]
    })
}

/// Asserts every channel is within its tolerance and returns the worst.
fn check_texels(
    label: &str,
    format: BcChannelFormat,
    actual: &[u32],
    expected: &[(u32, [u8; 4])],
) -> u8 {
    assert_eq!(actual.len(), expected.len(), "{label}: texel count");
    let mut worst = 0;
    for (texel, (&got, &(want, limit))) in actual.iter().zip(expected).enumerate() {
        let diff = distances(format, got, want);
        assert!(
            diff.iter().zip(limit).all(|(&d, l)| d <= l),
            "{label}: texel {texel} got {got:#010x}, want {want:#010x} (per-channel limit {limit:?})"
        );
        worst = worst.max(*diff.iter().max().unwrap());
    }
    worst
}

/// CPU reference words of `block` with their per-channel tolerances.
fn expected_block(
    case_format: BcChannelFormat,
    reference: fn(&[u8]) -> [u32; 16],
    block: &[u8],
) -> [(u32, [u8; 4]); 16] {
    let words = reference(block);
    let limits = tolerances(case_format, block);
    std::array::from_fn(|t| (words[t], limits[t]))
}

/// GPU reads of both layers: `[layer][mip]`.
type Reads = [[Vec<u32>; 2]; 2];

/// Uploads both layers, reads all four subresources on the GPU, compares
/// them with the CPU decoder and returns the reads with the worst difference.
fn read_and_compare(case: &Case) -> (Reads, u8) {
    let (bytes, row_pitch, slice_pitch) = upload_bytes(case.format, &case.layers);
    let upload = BcChannelUpload::new(
        &bytes,
        BcChannelUploadDesc {
            width: WIDTH,
            height: HEIGHT,
            mips: 2,
            layers: 2,
            format: case.format,
            row_pitch,
            slice_pitch,
        },
    )
    .unwrap();
    let gpu = context();
    let mut worst = 0;
    let reads = std::array::from_fn(|layer| {
        let decoded: Vec<_> = case.layers[layer]
            .iter()
            .map(|block| expected_block(case.format, case.reference, block))
            .collect();
        let levels = [
            crop(&decoded[..4], 2, WIDTH, HEIGHT),
            crop(&decoded[4..], 1, 2, 2),
        ];
        std::array::from_fn(|mip| {
            let actual = native(gpu.read_bc_channel_upload(&upload, layer, mip as u32));
            let expected = &levels[mip];
            let label = format!("{:?} layer {layer} mip {mip}", case.format);
            worst = worst.max(check_texels(&label, case.format, &actual, expected));
            actual
        })
    });
    for (layer, mip) in [(2, 0), (0, 2)] {
        assert_eq!(
            gpu.read_bc_channel_upload(&upload, layer, mip)
                .unwrap_err()
                .operation,
            "texture subresource"
        );
    }
    (reads, worst)
}

fn assert_near(label: &str, actual: f64, real: f64) {
    assert!(
        (actual - real).abs() <= REAL_TOLERANCE,
        "{label}: got {actual}, real value {real:.2}"
    );
}

fn byte(word: u32, channel: usize) -> u8 {
    word.to_le_bytes()[channel]
}

fn sbyte(word: u32, channel: usize) -> i8 {
    byte(word, channel) as i8
}

/// Unsigned blocks: eight-value (200 > 20), six-value (20 < 200), extremes
/// in both orders, and equal endpoints, which select the six-value mode.
fn unsigned_blocks() -> [[u8; 8]; 5] {
    [
        channel(200, 20),
        channel(20, 200),
        channel(128, 128),
        channel(255, 0),
        channel(0, 255),
    ]
}

/// Signed blocks: eight-value (100 > -100), six-value (-100 < 100), equal
/// zeros, and the extremes 127 and -128 in both orders.
fn signed_blocks() -> [[u8; 8]; 5] {
    [
        signed(100, -100),
        signed(-100, 100),
        signed(0, 0),
        signed(127, -128),
        signed(-128, 127),
    ]
}

/// Checks the hand-derived values of the base level of layer 0, whose
/// blocks are the five channel blocks in order.
fn unsigned_anchors(base: &[u32], channel: usize) {
    let v = |x: usize, y: usize| byte(base[y * WIDTH + x], channel);
    // Block 0, eight-value 200, 20: row 0 indices 6, 7, 0, 1, row 1 2..=5.
    assert_eq!([v(2, 0), v(3, 0)], [200, 20]);
    for (x, k) in (0..2).chain(0..4).zip([5, 6, 1, 2, 3, 4]) {
        let y = usize::from(k < 5);
        let real = f64::from((7 - k) * 200 + k * 20) / 7.0;
        assert_near(
            &format!("eight-value index {}", k + 1),
            v(x, y).into(),
            real,
        );
    }
    // Block 1, six-value 20, 200: column 4 holds indices 6, 2, 6, 2; index 6
    // is the fixed 0 and index 2 is (4 * 20 + 200) / 5 = 56.
    assert_eq!([v(4, 0), v(4, 2)], [0, 0]);
    assert_near("six-value index 2", v(4, 1).into(), 56.0);
    // Block 2, equal 128: row 4 indices 6, 7, 0, 1 -> fixed 0 and 255.
    assert_eq!([v(0, 4), v(1, 4), v(2, 4), v(3, 4)], [0, 255, 128, 128]);
    // Block 3, 255 > 0: texel (4, 4) is its texel 0, index 6:
    // (2 * 255) / 7 = 72.9.
    assert_near("extreme index 6", v(4, 4).into(), 510.0 / 7.0);
}

fn signed_anchors(base: &[u32], channel: usize) {
    let v = |x: usize, y: usize| sbyte(base[y * WIDTH + x], channel);
    // Block 0, eight-value 100, -100: row 0 indices 6, 7, 0, 1.
    assert_eq!([v(2, 0), v(3, 0)], [100, -100]);
    for (x, k) in (0..2).chain(0..4).zip([5, 6, 1, 2, 3, 4]) {
        let y = usize::from(k < 5);
        let real = f64::from((7 - k) * 100 - k * 100) / 7.0;
        assert_near(
            &format!("eight-value index {}", k + 1),
            v(x, y).into(),
            real,
        );
    }
    // Block 1, six-value -100, 100: column 4 holds indices 6, 2, 6, 2; index
    // 6 is the fixed -1.0 and index 2 is (4 * -100 + 100) / 5 = -60.
    assert_eq!([v(4, 0), v(4, 2)], [-127, -127]);
    assert_near("six-value index 2", v(4, 1).into(), -60.0);
    // Block 2, equal zeros: row 4 indices 6, 7, 0, 1 -> fixed -1.0, 1.0, 0, 0.
    assert_eq!([v(0, 4), v(1, 4), v(2, 4), v(3, 4)], [-127, 127, 0, 0]);
    // Block 3, 127 > -128: index 6 is (2 * 127 + 5 * -127) / 7 = -54.4, with
    // -128 read as -1.0.
    assert_near("extreme index 6", v(4, 4).into(), -381.0 / 7.0);
}

fn layers(
    blocks: [[u8; 8]; 5],
    other: [[u8; 8]; 5],
    pair: fn([u8; 8], [u8; 8]) -> Vec<u8>,
) -> [Layer; 2] {
    [
        blocks.map(|b| pair(b, other[0])),
        // Layer 1 rotates the blocks so each layer and mip differs.
        std::array::from_fn(|i| pair(blocks[(i + 2) % 5], other[(i + 1) % 5])),
    ]
}

fn single(block: [u8; 8], _: [u8; 8]) -> Vec<u8> {
    block.to_vec()
}

/// BC5 with the first block in red and a different block in green.
fn red_green(red: [u8; 8], green: [u8; 8]) -> Vec<u8> {
    dual(red, green)
}

#[test]
fn reads_pitched_bc4_unorm_across_mips_and_layers() {
    let blocks = unsigned_blocks();
    let case = Case {
        format: BcChannelFormat::Bc4Unorm,
        reference: bc4_unorm,
        layers: layers(blocks, blocks, single),
    };
    let (reads, worst) = read_and_compare(&case);
    let base = &reads[0][0];
    unsigned_anchors(base, 0);
    // A single channel reads as (red, 0, 0, 1).
    assert!(base.iter().all(|&word| word & 0xffff_ff00 == 0xff00_0000));
    println!("checked 58 BC4 UNORM texels, worst channel difference {worst}");
}

#[test]
fn reads_pitched_bc4_snorm_across_mips_and_layers() {
    let blocks = signed_blocks();
    let case = Case {
        format: BcChannelFormat::Bc4Snorm,
        reference: bc4_snorm,
        layers: layers(blocks, blocks, single),
    };
    let (reads, worst) = read_and_compare(&case);
    let base = &reads[0][0];
    signed_anchors(base, 0);
    assert!(base.iter().all(|&word| word & 0xffff_ff00 == 0x7f00_0000));
    // Layer 1 mip 1 is block 1 (-100 < 100, six-value): indices 6, 7, 2, 3.
    let small: Vec<_> = reads[1][1].iter().map(|&word| sbyte(word, 0)).collect();
    assert_eq!(small[..2], [-127, 127]);
    println!("checked 58 BC4 SNORM texels, worst channel difference {worst}");
}

#[test]
fn reads_pitched_bc5_unorm_in_red_green_order() {
    let blocks = unsigned_blocks();
    let case = Case {
        format: BcChannelFormat::Bc5Unorm,
        reference: bc5_unorm,
        layers: [
            blocks.map(|b| red_green(b, blocks[1])),
            layers(blocks, blocks, red_green)[1].clone(),
        ],
    };
    let (reads, worst) = read_and_compare(&case);
    let base = &reads[0][0];
    unsigned_anchors(base, 0);
    // Green holds the six-value block 1 throughout layer 0: (0, 0) is its
    // index 6, the fixed 0, and (1, 0) its index 7, the fixed 255.
    assert_eq!([byte(base[0], 1), byte(base[1], 1)], [0, 255]);
    assert!(base.iter().all(|&word| word & 0xffff_0000 == 0xff00_0000));
    println!("checked 58 BC5 UNORM texels, worst channel difference {worst}");
}

#[test]
fn reads_pitched_bc5_snorm_in_red_green_order() {
    let blocks = signed_blocks();
    let case = Case {
        format: BcChannelFormat::Bc5Snorm,
        reference: bc5_snorm,
        layers: [
            blocks.map(|b| red_green(b, blocks[1])),
            layers(blocks, blocks, red_green)[1].clone(),
        ],
    };
    let (reads, worst) = read_and_compare(&case);
    let base = &reads[0][0];
    signed_anchors(base, 0);
    // Green holds the six-value block 1 throughout layer 0: (0, 0) and
    // (1, 0) are its fixed -1.0 and 1.0.
    assert_eq!([sbyte(base[0], 1), sbyte(base[1], 1)], [-127, 127]);
    assert!(base.iter().all(|&word| word & 0xffff_0000 == 0x7f00_0000));
    println!("checked 58 BC5 SNORM texels, worst channel difference {worst}");
}

/// Reads a one-mip, one-layer upload of `width`×`height` from `blocks` with
/// one padding block per row, comparing with the CPU decoder.
fn read_edge(
    format: BcChannelFormat,
    reference: fn(&[u8]) -> [u32; 16],
    blocks: &[Vec<u8>],
    (width, height): (usize, usize),
) -> Vec<u32> {
    let block = format.block_bytes();
    let pad = PAD.repeat(block / PAD.len());
    let blocks_x = width.div_ceil(4);
    let mut bytes = Vec::new();
    for row in blocks.chunks(blocks_x) {
        for slot in row {
            bytes.extend_from_slice(slot);
        }
        bytes.extend_from_slice(&pad);
    }
    let upload = BcChannelUpload::new(
        &bytes,
        BcChannelUploadDesc {
            width,
            height,
            mips: 1,
            layers: 1,
            format,
            row_pitch: (blocks_x + 1) * block,
            slice_pitch: bytes.len(),
        },
    )
    .unwrap();
    let actual = native(context().read_bc_channel_upload(&upload, 0, 0));
    let decoded: Vec<_> = blocks
        .iter()
        .map(|block| expected_block(format, reference, block))
        .collect();
    let expected = crop(&decoded, blocks_x, width, height);
    check_texels(
        &format!("{format:?} {width}x{height}"),
        format,
        &actual,
        &expected,
    );
    actual
}

#[test]
fn reads_bc4_and_bc5_edge_blocks() {
    let [eight, six, ..] = unsigned_blocks();
    let [s_eight, s_six, _, _, s_low] = signed_blocks();
    // 5×3 is one row of two blocks (Metal's zero row stride), both cut at
    // row 3 and the second at column 1; 1×1 is one block cut to texel 0.
    // Row 0 of every block holds indices 6, 7, 0, 1; column 0 holds 6, 2, 6.
    let wide = read_edge(
        BcChannelFormat::Bc4Unorm,
        bc4_unorm,
        &[eight.to_vec(), six.to_vec()],
        (5, 3),
    );
    let red: Vec<_> = wide.iter().map(|&w| byte(w, 0)).collect();
    assert_eq!([red[2], red[3], red[4], red[14]], [200, 20, 0, 0]);
    assert_near("six-value index 2", red[9].into(), 56.0);
    let wide = read_edge(
        BcChannelFormat::Bc5Snorm,
        bc5_snorm,
        &[dual(s_eight, s_six), dual(s_six, s_eight)],
        (5, 3),
    );
    // Block 0: red eight-value endpoints 100, -100; green six-value 0 and 1
    // indices -100, 100. Block 1: red six-value fixed -1.0 at index 6, green
    // eight-value index 6 (2 * 100 + 5 * -100) / 7 = -42.9.
    let texel = |i: usize| [sbyte(wide[i], 0), sbyte(wide[i], 1)];
    assert_eq!([texel(2), texel(3)], [[100, -100], [-100, 100]]);
    assert_eq!(texel(4)[0], -127);
    assert_near(
        "green eight-value index 6",
        texel(4)[1].into(),
        -300.0 / 7.0,
    );
    // Index 6 of -128 < 127 is the six-value fixed -1.0.
    let one = read_edge(
        BcChannelFormat::Bc4Snorm,
        bc4_snorm,
        &[s_low.to_vec()],
        (1, 1),
    );
    assert_eq!(one, [0x7f00_0081]);
    let one = read_edge(
        BcChannelFormat::Bc5Unorm,
        bc5_unorm,
        &[dual(six, eight)],
        (1, 1),
    );
    assert_eq!(byte(one[0], 0), 0);
    assert_near(
        "green eight-value index 6",
        byte(one[0], 1).into(),
        500.0 / 7.0,
    );
    println!("checked 32 BC4 and BC5 texels from 5x3 and 1x1 edge levels");
}
