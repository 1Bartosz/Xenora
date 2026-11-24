// SPDX-License-Identifier: GPL-3.0-or-later
//! CPU block decoders for BC2, BC3, BC4 and BC5.
//!
//! Sources (public): Microsoft, "Texture Block Compression in Direct3D 11"
//! (sections BC2, BC3, BC4, BC5) and "Block Compression (Direct3D 10)"; Khronos
//! Data Format Specification 1.3, sections "S3TC compressed texture image
//! formats" and "RGTC compressed texture image formats".
//!
//! Colour blocks of BC2 and BC3 always use the four-colour interpolation,
//! whatever the order of their endpoints (Khronos, "S3TC compressed texture
//! image formats", DXT3/DXT5 subsections; the BC1 three-colour/alpha mode does
//! not apply). Texels are row-major, texel 0 at the top-left.
//!
//! Rounding: every interpolated value uses integer division that truncates
//! toward zero (the same rule `decode_bc1` uses). For unsigned values that is
//! floor. BC2 alpha nibbles expand as `v * 17`. The specifications define the
//! interpolation in real numbers; truncation is this crate's integer choice.
//!
//! SNORM: the specification maps both -128 and -127 to -1.0. Endpoints are
//! compared as the stored signed bytes; for interpolation -128 is treated as
//! -127. Output bytes are the stored endpoint bytes for indices 0 and 1 (so
//! -128 is preserved, not rewritten) and -127 / 127 for the fixed values.

use super::rgb565;

/// Reads texel `n` of a packed little-endian 48-bit array of 3-bit indices.
fn index3(bytes: &[u8; 6], n: usize) -> usize {
    let mut bits = 0u64;
    for (i, b) in bytes.iter().enumerate() {
        bits |= u64::from(*b) << (8 * i);
    }
    ((bits >> (3 * n)) & 7) as usize
}

fn unorm_palette(a0: u8, a1: u8) -> [u8; 8] {
    let (x, y) = (u16::from(a0), u16::from(a1));
    let mut p = [a0, a1, 0, 0, 0, 0, 0, 255];
    if a0 > a1 {
        for k in 1..=6u16 {
            p[(k + 1) as usize] = (((7 - k) * x + k * y) / 7) as u8;
        }
    } else {
        for k in 1..=4u16 {
            p[(k + 1) as usize] = (((5 - k) * x + k * y) / 5) as u8;
        }
        p[6] = 0;
    }
    p
}

fn snorm_palette(a0: i8, a1: i8) -> [i8; 8] {
    let x = i16::from(a0).max(-127);
    let y = i16::from(a1).max(-127);
    let mut p = [a0, a1, 0, 0, 0, 0, 0, 0];
    if a0 > a1 {
        for k in 1..=6i16 {
            p[(k + 1) as usize] = (((7 - k) * x + k * y) / 7) as i8;
        }
    } else {
        for k in 1..=4i16 {
            p[(k + 1) as usize] = (((5 - k) * x + k * y) / 5) as i8;
        }
        p[6] = -127;
        p[7] = 127;
    }
    p
}

fn unorm_block(block: &[u8]) -> [u8; 16] {
    let p = unorm_palette(block[0], block[1]);
    let idx: [u8; 6] = block[2..8].try_into().unwrap();
    std::array::from_fn(|t| p[index3(&idx, t)])
}

fn snorm_block(block: &[u8]) -> [i8; 16] {
    let p = snorm_palette(block[0] as i8, block[1] as i8);
    let idx: [u8; 6] = block[2..8].try_into().unwrap();
    std::array::from_fn(|t| p[index3(&idx, t)])
}

/// Decodes a BC4 UNORM block to 16 row-major red values.
pub fn decode_bc4_unorm(block: [u8; 8]) -> [u8; 16] {
    unorm_block(&block)
}

/// Decodes a BC4 SNORM block to 16 row-major signed red values.
pub fn decode_bc4_snorm(block: [u8; 8]) -> [i8; 16] {
    snorm_block(&block)
}

/// Decodes a BC5 UNORM block; each texel is `[red, green]` (first 8 bytes red).
pub fn decode_bc5_unorm(block: [u8; 16]) -> [[u8; 2]; 16] {
    let r = unorm_block(&block[..8]);
    let g = unorm_block(&block[8..]);
    std::array::from_fn(|t| [r[t], g[t]])
}

/// Decodes a BC5 SNORM block; each texel is `[red, green]`.
pub fn decode_bc5_snorm(block: [u8; 16]) -> [[i8; 2]; 16] {
    let r = snorm_block(&block[..8]);
    let g = snorm_block(&block[8..]);
    std::array::from_fn(|t| [r[t], g[t]])
}

/// Four-colour BC1-style palette, forced regardless of endpoint order.
fn colour_block(block: &[u8]) -> [[u8; 4]; 16] {
    let first = u16::from_le_bytes([block[0], block[1]]);
    let second = u16::from_le_bytes([block[2], block[3]]);
    let mut p = [rgb565(first), rgb565(second), [255; 4], [255; 4]];
    for c in 0..3 {
        let a = u16::from(p[0][c]);
        let b = u16::from(p[1][c]);
        p[2][c] = ((2 * a + b) / 3) as u8;
        p[3][c] = ((a + 2 * b) / 3) as u8;
    }
    let bits = u32::from_le_bytes([block[4], block[5], block[6], block[7]]);
    std::array::from_fn(|t| p[((bits >> (2 * t)) & 3) as usize])
}

/// Decodes a BC2 block to 16 row-major RGBA8 texels (explicit 4-bit alpha).
pub fn decode_bc2(block: [u8; 16]) -> [[u8; 4]; 16] {
    let mut out = colour_block(&block[8..]);
    for (t, px) in out.iter_mut().enumerate() {
        // Two texels per byte, low nibble first.
        px[3] = ((block[t / 2] >> (4 * (t % 2))) & 15) * 17;
    }
    out
}

/// Decodes a BC3 block to 16 row-major RGBA8 texels (interpolated alpha).
pub fn decode_bc3(block: [u8; 16]) -> [[u8; 4]; 16] {
    let mut out = colour_block(&block[8..]);
    let alpha = unorm_block(&block[..8]);
    for (px, a) in out.iter_mut().zip(alpha) {
        px[3] = a;
    }
    out
}
