// SPDX-License-Identifier: GPL-3.0-or-later
//! CPU block decoder for a bounded subset of BC6H.
//!
//! Source (public): Khronos Data Format Specification 1.3, section 20.2
//! "BC6H" (Tables 121 and 122, unquantization and final unquantization) and
//! section 20.1 (BC7 interpolation, Equation 2 and Table 119); Microsoft,
//! "BC6H Format" (Direct3D 11).
//!
//! Supported: the four single-subset modes, numbered as Khronos numbers
//! them (Microsoft's modes 11 to 14 in brackets): 3 (11, 10-bit endpoints),
//! 7 (12, 11-bit base, 9-bit deltas), 11 (13, 12-bit base, 8-bit deltas) and
//! 15 (14, 16-bit base, 4-bit deltas), for both signed and unsigned data.
//! Reserved modes 19, 23, 27 and 31 decode to zero as the specification
//! requires. The ten two-subset modes are rejected with
//! [`Bc6hError::UnsupportedMode`]; they are not implemented.
//!
//! Output texels are row-major `[r, g, b]` half-float bit patterns; the
//! format's alpha is always 1.0 and is not returned.
//!
//! Final unquantization follows the Khronos text: a negative interpolated
//! value keeps its sign bit even when its magnitude rounds to zero, so -1
//! becomes 0x8000 (-0.0). Microsoft's sample code returns 0x0000 there. This
//! is an open difference between the two public sources, not a hardware fact.

/// Khronos mode numbers this decoder implements.
pub const BC6H_SUPPORTED_MODES: [u8; 4] = [3, 7, 11, 15];

#[derive(Debug, PartialEq, Eq)]
pub enum Bc6hError {
    /// A two-subset mode, identified by its Khronos mode number.
    UnsupportedMode(u8),
}

/// Returns the block's Khronos mode number: the low two bits when they are
/// below two, otherwise the low five bits.
pub fn bc6h_mode(block: [u8; 16]) -> u8 {
    if block[0] & 3 < 2 {
        block[0] & 3
    } else {
        block[0] & 31
    }
}

/// Decodes an unsigned (UF16) BC6H block.
pub fn decode_bc6h_ufloat(block: [u8; 16]) -> Result<[[u16; 3]; 16], Bc6hError> {
    decode(block, false)
}

/// Decodes a signed (SF16) BC6H block.
pub fn decode_bc6h_sfloat(block: [u8; 16]) -> Result<[[u16; 3]; 16], Bc6hError> {
    decode(block, true)
}

/// Reads the block from its least significant bit upward.
struct Bits {
    bits: u128,
    offset: u32,
}

impl Bits {
    fn take(&mut self, count: u32) -> u32 {
        let value = (self.bits >> self.offset) as u32 & ((1u32 << count) - 1);
        self.offset += count;
        value
    }

    /// `v[high..=low]`, lowest block bit first into bit `low`.
    fn field(&mut self, target: &mut i32, high: u32, low: u32) {
        *target |= (self.take(high - low + 1) << low) as i32;
    }

    /// Table 122's reversed `v[a..b]` with `a < b`: the lowest block bit
    /// lands in bit `b` and each following bit one position lower.
    fn reversed(&mut self, target: &mut i32, low: u32, high: u32) {
        for bit in (low..=high).rev() {
            *target |= (self.take(1) << bit) as i32;
        }
    }
}

/// Endpoint bits, delta bits and whether E1 is stored relative to E0.
fn single_subset(mode: u8) -> Option<(u32, u32, bool)> {
    match mode {
        3 => Some((10, 10, false)),
        7 => Some((11, 9, true)),
        11 => Some((12, 8, true)),
        15 => Some((16, 4, true)),
        _ => None,
    }
}

fn sign_extend(value: i32, bits: u32) -> i32 {
    let shift = 32 - bits;
    (value << shift) >> shift
}

fn unquantize(x: i32, bits: u32, signed: bool) -> i32 {
    if !signed {
        if bits >= 15 {
            x
        } else if x == 0 {
            0
        } else if x == (1 << bits) - 1 {
            0xffff
        } else {
            ((x << 15) + 0x4000) >> (bits - 1)
        }
    } else if bits >= 16 {
        x
    } else {
        let magnitude = x.abs();
        let unq = if magnitude == 0 {
            0
        } else if magnitude >= (1 << (bits - 1)) - 1 {
            0x7fff
        } else {
            ((magnitude << 15) + 0x4000) >> (bits - 1)
        };
        if x < 0 { -unq } else { unq }
    }
}

fn finish(i: i32, signed: bool) -> u16 {
    if !signed {
        ((i * 31) >> 6) as u16
    } else if i < 0 {
        ((((-i) * 31) >> 5) | 0x8000) as u16
    } else {
        ((i * 31) >> 5) as u16
    }
}

const WEIGHTS: [i32; 16] = [0, 4, 9, 13, 17, 21, 26, 30, 34, 38, 43, 47, 51, 55, 60, 64];

fn decode(block: [u8; 16], signed: bool) -> Result<[[u16; 3]; 16], Bc6hError> {
    let mode = bc6h_mode(block);
    if matches!(mode, 19 | 23 | 27 | 31) {
        return Ok([[0; 3]; 16]);
    }
    let (epb, delta, transformed) = single_subset(mode).ok_or(Bc6hError::UnsupportedMode(mode))?;
    let mut bits = Bits {
        bits: u128::from_le_bytes(block),
        offset: 5,
    };
    let mut e0 = [0i32; 3];
    let mut e1 = [0i32; 3];
    for channel in &mut e0 {
        bits.field(channel, 9, 0);
    }
    for c in 0..3 {
        match mode {
            3 => bits.field(&mut e1[c], 9, 0),
            7 => {
                bits.field(&mut e1[c], 8, 0);
                bits.field(&mut e0[c], 10, 10);
            }
            11 => {
                bits.field(&mut e1[c], 7, 0);
                bits.reversed(&mut e0[c], 10, 11);
            }
            _ => {
                bits.field(&mut e1[c], 3, 0);
                bits.reversed(&mut e0[c], 10, 15);
            }
        }
    }
    debug_assert_eq!(bits.offset, 65);
    let mut ends = [[0i32; 3]; 2];
    for c in 0..3 {
        let mut a = e0[c];
        let mut b = e1[c];
        if signed {
            a = sign_extend(a, epb);
        }
        if transformed || signed {
            b = sign_extend(b, delta);
        }
        if transformed {
            b = (a + b) & ((1 << epb) - 1);
            if signed {
                b = sign_extend(b, epb);
            }
        }
        ends[0][c] = unquantize(a, epb, signed);
        ends[1][c] = unquantize(b, epb, signed);
    }
    let mut texels = [[0u16; 3]; 16];
    for (texel, out) in texels.iter_mut().enumerate() {
        // Texel 0 is the anchor: its implicit high index bit is zero.
        let index = bits.take(if texel == 0 { 3 } else { 4 }) as usize;
        let w = WEIGHTS[index];
        for c in 0..3 {
            let i = ((64 - w) * ends[0][c] + w * ends[1][c] + 32) >> 6;
            out[c] = finish(i, signed);
        }
    }
    Ok(texels)
}
