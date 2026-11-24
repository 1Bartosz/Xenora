// SPDX-License-Identifier: GPL-3.0-or-later
//! CPU block decoder for BC7 (BPTC RGBA, unsigned normalized).
//!
//! Source (public): Khronos Data Format Specification 1.3, section 20.1
//! "BC7": Table 109 (mode parameters), Table 111 (bit layout), the endpoint
//! P-bit and bit-replication text, Tables 114-118 (partitions and anchor
//! indices), the index-offset formula, Equation 2 with Table 119 (weights)
//! and Table 120 (rotation). Microsoft's "BC7 Format" pages describe the same
//! format. The partition and anchor tables below were extracted mechanically
//! from the Khronos HTML and checked for consistency (every anchor lies in its
//! subset).
//!
//! All eight modes are decoded. A block whose low byte is zero has no mode;
//! the specification says decoders return 0 for every channel of every texel,
//! and [`decode_bc7`] does so. [`bc7_mode`] tells such blocks apart.
//!
//! Output is the 8-bit value of each channel, row-major, texel 0 at the
//! top-left. The sRGB variant uses the same bytes with a different transfer
//! function, which is outside this decoder.

/// Modes this decoder implements (Khronos/Microsoft numbering).
pub const BC7_SUPPORTED_MODES: [u8; 8] = [0, 1, 2, 3, 4, 5, 6, 7];

/// Table 109, one row per mode.
struct Mode {
    subsets: usize,
    partition_bits: u32,
    rotation_bits: u32,
    index_selection_bits: u32,
    color_bits: u32,
    alpha_bits: u32,
    endpoint_pbits: bool,
    shared_pbits: bool,
    index_bits: u32,
    index2_bits: u32,
}

#[rustfmt::skip]
const MODES: [Mode; 8] = [
    Mode { subsets: 3, partition_bits: 4, rotation_bits: 0, index_selection_bits: 0, color_bits: 4, alpha_bits: 0, endpoint_pbits: true,  shared_pbits: false, index_bits: 3, index2_bits: 0 },
    Mode { subsets: 2, partition_bits: 6, rotation_bits: 0, index_selection_bits: 0, color_bits: 6, alpha_bits: 0, endpoint_pbits: false, shared_pbits: true,  index_bits: 3, index2_bits: 0 },
    Mode { subsets: 3, partition_bits: 6, rotation_bits: 0, index_selection_bits: 0, color_bits: 5, alpha_bits: 0, endpoint_pbits: false, shared_pbits: false, index_bits: 2, index2_bits: 0 },
    Mode { subsets: 2, partition_bits: 6, rotation_bits: 0, index_selection_bits: 0, color_bits: 7, alpha_bits: 0, endpoint_pbits: true,  shared_pbits: false, index_bits: 2, index2_bits: 0 },
    Mode { subsets: 1, partition_bits: 0, rotation_bits: 2, index_selection_bits: 1, color_bits: 5, alpha_bits: 6, endpoint_pbits: false, shared_pbits: false, index_bits: 2, index2_bits: 3 },
    Mode { subsets: 1, partition_bits: 0, rotation_bits: 2, index_selection_bits: 0, color_bits: 7, alpha_bits: 8, endpoint_pbits: false, shared_pbits: false, index_bits: 2, index2_bits: 2 },
    Mode { subsets: 1, partition_bits: 0, rotation_bits: 0, index_selection_bits: 0, color_bits: 7, alpha_bits: 7, endpoint_pbits: true,  shared_pbits: false, index_bits: 4, index2_bits: 0 },
    Mode { subsets: 2, partition_bits: 6, rotation_bits: 0, index_selection_bits: 0, color_bits: 5, alpha_bits: 5, endpoint_pbits: true,  shared_pbits: false, index_bits: 2, index2_bits: 0 },
];

/// Table 119 weights for 2-, 3- and 4-bit indices.
const WEIGHTS2: [u16; 4] = [0, 21, 43, 64];
const WEIGHTS3: [u16; 8] = [0, 9, 18, 27, 37, 46, 55, 64];
const WEIGHTS4: [u16; 16] = [0, 4, 9, 13, 17, 21, 26, 30, 34, 38, 43, 47, 51, 55, 60, 64];

/// Table 114: subset of each texel (row-major) for the 64 two-subset partitions.
const PARTITION2: [[u8; 16]; 64] = [
    [0, 0, 1, 1, 0, 0, 1, 1, 0, 0, 1, 1, 0, 0, 1, 1], // 0
    [0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 1], // 1
    [0, 1, 1, 1, 0, 1, 1, 1, 0, 1, 1, 1, 0, 1, 1, 1], // 2
    [0, 0, 0, 1, 0, 0, 1, 1, 0, 0, 1, 1, 0, 1, 1, 1], // 3
    [0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 1, 1], // 4
    [0, 0, 1, 1, 0, 1, 1, 1, 0, 1, 1, 1, 1, 1, 1, 1], // 5
    [0, 0, 0, 1, 0, 0, 1, 1, 0, 1, 1, 1, 1, 1, 1, 1], // 6
    [0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 1, 1, 0, 1, 1, 1], // 7
    [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 1, 1], // 8
    [0, 0, 1, 1, 0, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1], // 9
    [0, 0, 0, 0, 0, 0, 0, 1, 0, 1, 1, 1, 1, 1, 1, 1], // 10
    [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 1, 1, 1], // 11
    [0, 0, 0, 1, 0, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1], // 12
    [0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 1, 1, 1, 1], // 13
    [0, 0, 0, 0, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1], // 14
    [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1], // 15
    [0, 0, 0, 0, 1, 0, 0, 0, 1, 1, 1, 0, 1, 1, 1, 1], // 16
    [0, 1, 1, 1, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0], // 17
    [0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 1, 1, 1, 0], // 18
    [0, 1, 1, 1, 0, 0, 1, 1, 0, 0, 0, 1, 0, 0, 0, 0], // 19
    [0, 0, 1, 1, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0], // 20
    [0, 0, 0, 0, 1, 0, 0, 0, 1, 1, 0, 0, 1, 1, 1, 0], // 21
    [0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 1, 1, 0, 0], // 22
    [0, 1, 1, 1, 0, 0, 1, 1, 0, 0, 1, 1, 0, 0, 0, 1], // 23
    [0, 0, 1, 1, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 0], // 24
    [0, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 1, 1, 0, 0], // 25
    [0, 1, 1, 0, 0, 1, 1, 0, 0, 1, 1, 0, 0, 1, 1, 0], // 26
    [0, 0, 1, 1, 0, 1, 1, 0, 0, 1, 1, 0, 1, 1, 0, 0], // 27
    [0, 0, 0, 1, 0, 1, 1, 1, 1, 1, 1, 0, 1, 0, 0, 0], // 28
    [0, 0, 0, 0, 1, 1, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0], // 29
    [0, 1, 1, 1, 0, 0, 0, 1, 1, 0, 0, 0, 1, 1, 1, 0], // 30
    [0, 0, 1, 1, 1, 0, 0, 1, 1, 0, 0, 1, 1, 1, 0, 0], // 31
    [0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1], // 32
    [0, 0, 0, 0, 1, 1, 1, 1, 0, 0, 0, 0, 1, 1, 1, 1], // 33
    [0, 1, 0, 1, 1, 0, 1, 0, 0, 1, 0, 1, 1, 0, 1, 0], // 34
    [0, 0, 1, 1, 0, 0, 1, 1, 1, 1, 0, 0, 1, 1, 0, 0], // 35
    [0, 0, 1, 1, 1, 1, 0, 0, 0, 0, 1, 1, 1, 1, 0, 0], // 36
    [0, 1, 0, 1, 0, 1, 0, 1, 1, 0, 1, 0, 1, 0, 1, 0], // 37
    [0, 1, 1, 0, 1, 0, 0, 1, 0, 1, 1, 0, 1, 0, 0, 1], // 38
    [0, 1, 0, 1, 1, 0, 1, 0, 1, 0, 1, 0, 0, 1, 0, 1], // 39
    [0, 1, 1, 1, 0, 0, 1, 1, 1, 1, 0, 0, 1, 1, 1, 0], // 40
    [0, 0, 0, 1, 0, 0, 1, 1, 1, 1, 0, 0, 1, 0, 0, 0], // 41
    [0, 0, 1, 1, 0, 0, 1, 0, 0, 1, 0, 0, 1, 1, 0, 0], // 42
    [0, 0, 1, 1, 1, 0, 1, 1, 1, 1, 0, 1, 1, 1, 0, 0], // 43
    [0, 1, 1, 0, 1, 0, 0, 1, 1, 0, 0, 1, 0, 1, 1, 0], // 44
    [0, 0, 1, 1, 1, 1, 0, 0, 1, 1, 0, 0, 0, 0, 1, 1], // 45
    [0, 1, 1, 0, 0, 1, 1, 0, 1, 0, 0, 1, 1, 0, 0, 1], // 46
    [0, 0, 0, 0, 0, 1, 1, 0, 0, 1, 1, 0, 0, 0, 0, 0], // 47
    [0, 1, 0, 0, 1, 1, 1, 0, 0, 1, 0, 0, 0, 0, 0, 0], // 48
    [0, 0, 1, 0, 0, 1, 1, 1, 0, 0, 1, 0, 0, 0, 0, 0], // 49
    [0, 0, 0, 0, 0, 0, 1, 0, 0, 1, 1, 1, 0, 0, 1, 0], // 50
    [0, 0, 0, 0, 0, 1, 0, 0, 1, 1, 1, 0, 0, 1, 0, 0], // 51
    [0, 1, 1, 0, 1, 1, 0, 0, 1, 0, 0, 1, 0, 0, 1, 1], // 52
    [0, 0, 1, 1, 0, 1, 1, 0, 1, 1, 0, 0, 1, 0, 0, 1], // 53
    [0, 1, 1, 0, 0, 0, 1, 1, 1, 0, 0, 1, 1, 1, 0, 0], // 54
    [0, 0, 1, 1, 1, 0, 0, 1, 1, 1, 0, 0, 0, 1, 1, 0], // 55
    [0, 1, 1, 0, 1, 1, 0, 0, 1, 1, 0, 0, 1, 0, 0, 1], // 56
    [0, 1, 1, 0, 0, 0, 1, 1, 0, 0, 1, 1, 1, 0, 0, 1], // 57
    [0, 1, 1, 1, 1, 1, 1, 0, 1, 0, 0, 0, 0, 0, 0, 1], // 58
    [0, 0, 0, 1, 1, 0, 0, 0, 1, 1, 1, 0, 0, 1, 1, 1], // 59
    [0, 0, 0, 0, 1, 1, 1, 1, 0, 0, 1, 1, 0, 0, 1, 1], // 60
    [0, 0, 1, 1, 0, 0, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0], // 61
    [0, 0, 1, 0, 0, 0, 1, 0, 1, 1, 1, 0, 1, 1, 1, 0], // 62
    [0, 1, 0, 0, 0, 1, 0, 0, 0, 1, 1, 1, 0, 1, 1, 1], // 63
];

/// Table 115: subset of each texel (row-major) for the 64 three-subset partitions.
const PARTITION3: [[u8; 16]; 64] = [
    [0, 0, 1, 1, 0, 0, 1, 1, 0, 2, 2, 1, 2, 2, 2, 2], // 0
    [0, 0, 0, 1, 0, 0, 1, 1, 2, 2, 1, 1, 2, 2, 2, 1], // 1
    [0, 0, 0, 0, 2, 0, 0, 1, 2, 2, 1, 1, 2, 2, 1, 1], // 2
    [0, 2, 2, 2, 0, 0, 2, 2, 0, 0, 1, 1, 0, 1, 1, 1], // 3
    [0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 2, 2, 1, 1, 2, 2], // 4
    [0, 0, 1, 1, 0, 0, 1, 1, 0, 0, 2, 2, 0, 0, 2, 2], // 5
    [0, 0, 2, 2, 0, 0, 2, 2, 1, 1, 1, 1, 1, 1, 1, 1], // 6
    [0, 0, 1, 1, 0, 0, 1, 1, 2, 2, 1, 1, 2, 2, 1, 1], // 7
    [0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2], // 8
    [0, 0, 0, 0, 1, 1, 1, 1, 1, 1, 1, 1, 2, 2, 2, 2], // 9
    [0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 2, 2, 2, 2], // 10
    [0, 0, 1, 2, 0, 0, 1, 2, 0, 0, 1, 2, 0, 0, 1, 2], // 11
    [0, 1, 1, 2, 0, 1, 1, 2, 0, 1, 1, 2, 0, 1, 1, 2], // 12
    [0, 1, 2, 2, 0, 1, 2, 2, 0, 1, 2, 2, 0, 1, 2, 2], // 13
    [0, 0, 1, 1, 0, 1, 1, 2, 1, 1, 2, 2, 1, 2, 2, 2], // 14
    [0, 0, 1, 1, 2, 0, 0, 1, 2, 2, 0, 0, 2, 2, 2, 0], // 15
    [0, 0, 0, 1, 0, 0, 1, 1, 0, 1, 1, 2, 1, 1, 2, 2], // 16
    [0, 1, 1, 1, 0, 0, 1, 1, 2, 0, 0, 1, 2, 2, 0, 0], // 17
    [0, 0, 0, 0, 1, 1, 2, 2, 1, 1, 2, 2, 1, 1, 2, 2], // 18
    [0, 0, 2, 2, 0, 0, 2, 2, 0, 0, 2, 2, 1, 1, 1, 1], // 19
    [0, 1, 1, 1, 0, 1, 1, 1, 0, 2, 2, 2, 0, 2, 2, 2], // 20
    [0, 0, 0, 1, 0, 0, 0, 1, 2, 2, 2, 1, 2, 2, 2, 1], // 21
    [0, 0, 0, 0, 0, 0, 1, 1, 0, 1, 2, 2, 0, 1, 2, 2], // 22
    [0, 0, 0, 0, 1, 1, 0, 0, 2, 2, 1, 0, 2, 2, 1, 0], // 23
    [0, 1, 2, 2, 0, 1, 2, 2, 0, 0, 1, 1, 0, 0, 0, 0], // 24
    [0, 0, 1, 2, 0, 0, 1, 2, 1, 1, 2, 2, 2, 2, 2, 2], // 25
    [0, 1, 1, 0, 1, 2, 2, 1, 1, 2, 2, 1, 0, 1, 1, 0], // 26
    [0, 0, 0, 0, 0, 1, 1, 0, 1, 2, 2, 1, 1, 2, 2, 1], // 27
    [0, 0, 2, 2, 1, 1, 0, 2, 1, 1, 0, 2, 0, 0, 2, 2], // 28
    [0, 1, 1, 0, 0, 1, 1, 0, 2, 0, 0, 2, 2, 2, 2, 2], // 29
    [0, 0, 1, 1, 0, 1, 2, 2, 0, 1, 2, 2, 0, 0, 1, 1], // 30
    [0, 0, 0, 0, 2, 0, 0, 0, 2, 2, 1, 1, 2, 2, 2, 1], // 31
    [0, 0, 0, 0, 0, 0, 0, 2, 1, 1, 2, 2, 1, 2, 2, 2], // 32
    [0, 2, 2, 2, 0, 0, 2, 2, 0, 0, 1, 2, 0, 0, 1, 1], // 33
    [0, 0, 1, 1, 0, 0, 1, 2, 0, 0, 2, 2, 0, 2, 2, 2], // 34
    [0, 1, 2, 0, 0, 1, 2, 0, 0, 1, 2, 0, 0, 1, 2, 0], // 35
    [0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 0, 0, 0, 0], // 36
    [0, 1, 2, 0, 1, 2, 0, 1, 2, 0, 1, 2, 0, 1, 2, 0], // 37
    [0, 1, 2, 0, 2, 0, 1, 2, 1, 2, 0, 1, 0, 1, 2, 0], // 38
    [0, 0, 1, 1, 2, 2, 0, 0, 1, 1, 2, 2, 0, 0, 1, 1], // 39
    [0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 0, 0, 0, 0, 1, 1], // 40
    [0, 1, 0, 1, 0, 1, 0, 1, 2, 2, 2, 2, 2, 2, 2, 2], // 41
    [0, 0, 0, 0, 0, 0, 0, 0, 2, 1, 2, 1, 2, 1, 2, 1], // 42
    [0, 0, 2, 2, 1, 1, 2, 2, 0, 0, 2, 2, 1, 1, 2, 2], // 43
    [0, 0, 2, 2, 0, 0, 1, 1, 0, 0, 2, 2, 0, 0, 1, 1], // 44
    [0, 2, 2, 0, 1, 2, 2, 1, 0, 2, 2, 0, 1, 2, 2, 1], // 45
    [0, 1, 0, 1, 2, 2, 2, 2, 2, 2, 2, 2, 0, 1, 0, 1], // 46
    [0, 0, 0, 0, 2, 1, 2, 1, 2, 1, 2, 1, 2, 1, 2, 1], // 47
    [0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 2, 2, 2, 2], // 48
    [0, 2, 2, 2, 0, 1, 1, 1, 0, 2, 2, 2, 0, 1, 1, 1], // 49
    [0, 0, 0, 2, 1, 1, 1, 2, 0, 0, 0, 2, 1, 1, 1, 2], // 50
    [0, 0, 0, 0, 2, 1, 1, 2, 2, 1, 1, 2, 2, 1, 1, 2], // 51
    [0, 2, 2, 2, 0, 1, 1, 1, 0, 1, 1, 1, 0, 2, 2, 2], // 52
    [0, 0, 0, 2, 1, 1, 1, 2, 1, 1, 1, 2, 0, 0, 0, 2], // 53
    [0, 1, 1, 0, 0, 1, 1, 0, 0, 1, 1, 0, 2, 2, 2, 2], // 54
    [0, 0, 0, 0, 0, 0, 0, 0, 2, 1, 1, 2, 2, 1, 1, 2], // 55
    [0, 1, 1, 0, 0, 1, 1, 0, 2, 2, 2, 2, 2, 2, 2, 2], // 56
    [0, 0, 2, 2, 0, 0, 1, 1, 0, 0, 1, 1, 0, 0, 2, 2], // 57
    [0, 0, 2, 2, 1, 1, 2, 2, 1, 1, 2, 2, 0, 0, 2, 2], // 58
    [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2, 1, 1, 2], // 59
    [0, 0, 0, 2, 0, 0, 0, 1, 0, 0, 0, 2, 0, 0, 0, 1], // 60
    [0, 2, 2, 2, 1, 2, 2, 2, 0, 2, 2, 2, 1, 2, 2, 2], // 61
    [0, 1, 0, 1, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2], // 62
    [0, 1, 1, 1, 2, 0, 1, 1, 2, 2, 0, 1, 2, 2, 2, 0], // 63
];

/// Table 118: anchor index of subset 1, two-subset partitions.
const ANCHOR2: [u8; 64] = [
    15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 2, 8, 2, 2, 8, 8, 15, 2, 8,
    2, 2, 8, 8, 2, 2, 15, 15, 6, 8, 2, 8, 15, 15, 2, 8, 2, 2, 2, 15, 15, 6, 6, 2, 6, 8, 15, 15, 2,
    2, 15, 15, 15, 15, 15, 2, 2, 15,
];

/// Table 116: anchor index of subset 1, three-subset partitions.
const ANCHOR3_2: [u8; 64] = [
    3, 3, 15, 15, 8, 3, 15, 15, 8, 8, 6, 6, 6, 5, 3, 3, 3, 3, 8, 15, 3, 3, 6, 10, 5, 8, 8, 6, 8, 5,
    15, 15, 8, 15, 3, 5, 6, 10, 8, 15, 15, 3, 15, 5, 15, 15, 15, 15, 3, 15, 5, 5, 5, 8, 5, 10, 5,
    10, 8, 13, 15, 12, 3, 3,
];

/// Table 117: anchor index of subset 2, three-subset partitions.
const ANCHOR3_3: [u8; 64] = [
    15, 8, 8, 3, 15, 15, 3, 8, 15, 15, 15, 15, 15, 15, 15, 8, 15, 8, 15, 3, 15, 8, 15, 8, 3, 15, 6,
    10, 15, 15, 10, 8, 15, 3, 15, 10, 10, 8, 9, 10, 6, 15, 8, 15, 3, 6, 6, 8, 15, 3, 15, 15, 15,
    15, 15, 15, 15, 15, 15, 15, 3, 15, 15, 8,
];

/// Reads the block's bits in increasing order, starting at bit 0 of byte 0.
struct Bits {
    value: u128,
    pos: u32,
}

impl Bits {
    fn read(&mut self, n: u32) -> u32 {
        let v = if n == 0 {
            0
        } else {
            ((self.value >> self.pos) & ((1u128 << n) - 1)) as u32
        };
        self.pos += n;
        v
    }
}

/// Returns the block's mode (the position of the lowest set bit of byte 0),
/// or `None` when byte 0 is zero, which the specification reserves.
pub fn bc7_mode(block: &[u8; 16]) -> Option<u8> {
    (block[0] != 0).then(|| block[0].trailing_zeros() as u8)
}

/// Appends the P-bit (when present) below the stored bits and fills the rest
/// of the byte by replicating the top bits.
fn expand(stored: u32, bits: u32, pbit: Option<u32>) -> u8 {
    let (v, width) = match pbit {
        Some(p) => ((stored << 1) | p, bits + 1),
        None => (stored, bits),
    };
    let high = v << (8 - width);
    (high | (high >> width)) as u8
}

fn weights(bits: u32) -> &'static [u16] {
    match bits {
        2 => &WEIGHTS2,
        3 => &WEIGHTS3,
        _ => &WEIGHTS4,
    }
}

/// Equation 2.
fn interpolate(e0: u8, e1: u8, weight: u16) -> u8 {
    (((64 - weight) * u16::from(e0) + weight * u16::from(e1) + 32) >> 6) as u8
}

/// Decodes one BC7 block to sixteen RGBA8 texels, row-major.
pub fn decode_bc7(block: [u8; 16]) -> [[u8; 4]; 16] {
    let Some(mode_number) = bc7_mode(&block) else {
        return [[0; 4]; 16];
    };
    let mode = &MODES[usize::from(mode_number)];
    let mut bits = Bits {
        value: u128::from_le_bytes(block),
        pos: u32::from(mode_number) + 1,
    };
    let partition = bits.read(mode.partition_bits) as usize;
    let rotation = bits.read(mode.rotation_bits);
    let index_selection = bits.read(mode.index_selection_bits);

    // Endpoint fields: channel-major, then subset, then endpoint.
    let endpoints = 2 * mode.subsets;
    let mut raw = [[0u32; 4]; 6];
    for channel in 0..3 {
        for e in raw.iter_mut().take(endpoints) {
            e[channel] = bits.read(mode.color_bits);
        }
    }
    for e in raw.iter_mut().take(endpoints) {
        e[3] = bits.read(mode.alpha_bits);
    }
    let mut pbits = [None; 6];
    if mode.endpoint_pbits {
        for p in pbits.iter_mut().take(endpoints) {
            *p = Some(bits.read(1));
        }
    }
    if mode.shared_pbits {
        for subset in 0..mode.subsets {
            let p = bits.read(1);
            pbits[2 * subset] = Some(p);
            pbits[2 * subset + 1] = Some(p);
        }
    }
    let mut colors = [[0u8; 4]; 6];
    for (e, color) in colors.iter_mut().enumerate().take(endpoints) {
        for channel in 0..3 {
            color[channel] = expand(raw[e][channel], mode.color_bits, pbits[e]);
        }
        color[3] = if mode.alpha_bits == 0 {
            255
        } else {
            expand(raw[e][3], mode.alpha_bits, pbits[e])
        };
    }

    let subset_of = |texel: usize| -> usize {
        match mode.subsets {
            2 => usize::from(PARTITION2[partition][texel]),
            3 => usize::from(PARTITION3[partition][texel]),
            _ => 0,
        }
    };
    let is_anchor = |texel: usize| -> bool {
        texel == 0
            || match mode.subsets {
                2 => texel == usize::from(ANCHOR2[partition]),
                3 => {
                    texel == usize::from(ANCHOR3_2[partition])
                        || texel == usize::from(ANCHOR3_3[partition])
                }
                _ => false,
            }
    };
    let mut read_indices = |width: u32| -> [u32; 16] {
        let mut out = [0u32; 16];
        if width > 0 {
            for (texel, index) in out.iter_mut().enumerate() {
                *index = bits.read(width - u32::from(is_anchor(texel)));
            }
        }
        out
    };
    let primary = read_indices(mode.index_bits);
    let secondary = read_indices(mode.index2_bits);

    // Index selection: colour from the secondary indices when the bit is one;
    // alpha from the secondary indices when they exist and the bit is zero.
    let (color_index, color_bits, alpha_index, alpha_bits) = if mode.index2_bits == 0 {
        (primary, mode.index_bits, primary, mode.index_bits)
    } else if index_selection == 1 {
        (secondary, mode.index2_bits, primary, mode.index_bits)
    } else {
        (primary, mode.index_bits, secondary, mode.index2_bits)
    };

    let mut out = [[0u8; 4]; 16];
    for (texel, px) in out.iter_mut().enumerate() {
        let subset = subset_of(texel);
        let (e0, e1) = (colors[2 * subset], colors[2 * subset + 1]);
        let wc = weights(color_bits)[color_index[texel] as usize];
        let wa = weights(alpha_bits)[alpha_index[texel] as usize];
        for channel in 0..3 {
            px[channel] = interpolate(e0[channel], e1[channel], wc);
        }
        px[3] = interpolate(e0[3], e1[3], wa);
        // Table 120.
        if rotation > 0 {
            px.swap(3, rotation as usize - 1);
        }
    }
    out
}
