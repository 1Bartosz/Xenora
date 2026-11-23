// SPDX-License-Identifier: GPL-3.0-or-later

//! Independent pure-Rust CPU reference for deterministic RGBA8 2D texture sampling.
//!
//! # Coordinate Mapping and Filtering
//!
//! Sampling operates on normalized `(u, v)` coordinates using nearest-neighbor filtering
//! and clamp-to-edge addressing:
//!
//! 1. Non-finite coordinates (`NaN`, `+Inf`, `-Inf`) are rejected with [`SamplingError::NonFiniteCoordinates`].
//! 2. Coordinates are clamped to the unit interval:
//!    `u_clamped = u.clamp(0.0, 1.0)`
//!    `v_clamped = v.clamp(0.0, 1.0)`
//! 3. For a mip level with dimensions `(W, H)`:
//!    `x = min(floor(u_clamped * W), W - 1)`
//!    `y = min(floor(v_clamped * H), H - 1)`
//!    At exact texel boundaries where `u * W` is an integer, standard IEEE 754 `floor`
//!    deterministically assigns the boundary to the right/lower texel (for example, with `W = 2`,
//!    `u = 0.5` maps to `x = 1`), while `u = 1.0` maps to `x = 1` via edge clamping.

use std::fmt;

/// Bytes per RGBA8 texel.
pub const RGBA8_TEXEL_BYTES: usize = 4;

/// Errors produced during CPU texture sampling reference execution.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SamplingError {
    /// Texture width or height is zero or exceeds limits.
    InvalidDimensions,
    /// Mip count is zero or exceeds maximum possible for dimensions.
    InvalidMipCount,
    /// Specified mip level index is out of bounds.
    MipOutOfBounds,
    /// Row pitch is smaller than tight row bytes (`width * 4`) or unaligned.
    InvalidRowPitch,
    /// Buffer offset causes an overflow or unaligned read.
    InvalidOffset,
    /// Underlying data buffer is smaller than required by descriptor layout.
    BufferTooSmall,
    /// Input coordinates contained NaN or infinity.
    NonFiniteCoordinates,
}

impl fmt::Display for SamplingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidDimensions => write!(f, "invalid texture dimensions"),
            Self::InvalidMipCount => write!(f, "invalid mip count"),
            Self::MipOutOfBounds => write!(f, "selected mip index is out of bounds"),
            Self::InvalidRowPitch => write!(f, "invalid row pitch for mip level"),
            Self::InvalidOffset => write!(f, "invalid buffer offset"),
            Self::BufferTooSmall => write!(f, "buffer is too small for texture layout"),
            Self::NonFiniteCoordinates => write!(f, "non-finite texture coordinates"),
        }
    }
}

impl std::error::Error for SamplingError {}

/// Mip-level layout specification for 2D RGBA8 sampling.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MipLayout {
    pub width: usize,
    pub height: usize,
    pub offset: usize,
    pub row_pitch: usize,
}

/// Description of a 2D RGBA8 texture with an explicit mip chain.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rgba8TextureDesc {
    pub width: usize,
    pub height: usize,
    pub mips: u32,
    pub mip_layouts: Vec<MipLayout>,
}

impl Rgba8TextureDesc {
    /// Create a descriptor with tightly packed row-major mip levels consecutively arranged.
    pub fn new_packed(width: usize, height: usize, mips: u32) -> Result<Self, SamplingError> {
        let max_mips = max_mip_levels(width, height)?;
        if mips == 0 || mips > max_mips {
            return Err(SamplingError::InvalidMipCount);
        }

        let mut mip_layouts = Vec::with_capacity(mips as usize);
        let mut current_offset = 0usize;

        for mip in 0..mips {
            let mip_w = (width >> mip).max(1);
            let mip_h = (height >> mip).max(1);
            let row_pitch = mip_w
                .checked_mul(RGBA8_TEXEL_BYTES)
                .ok_or(SamplingError::InvalidDimensions)?;
            let level_bytes = row_pitch
                .checked_mul(mip_h)
                .ok_or(SamplingError::InvalidDimensions)?;

            mip_layouts.push(MipLayout {
                width: mip_w,
                height: mip_h,
                offset: current_offset,
                row_pitch,
            });

            current_offset = current_offset
                .checked_add(level_bytes)
                .ok_or(SamplingError::InvalidDimensions)?;
        }

        Ok(Self {
            width,
            height,
            mips,
            mip_layouts,
        })
    }

    /// Validates internal consistency and returns the total required byte size.
    pub fn required_size(&self) -> Result<usize, SamplingError> {
        let max_mips = max_mip_levels(self.width, self.height)?;
        if self.mips == 0 || self.mips > max_mips || self.mip_layouts.len() != self.mips as usize {
            return Err(SamplingError::InvalidMipCount);
        }

        let mut total_end = 0usize;
        for (i, layout) in self.mip_layouts.iter().enumerate() {
            let expected_w = (self.width >> i).max(1);
            let expected_h = (self.height >> i).max(1);
            if layout.width != expected_w || layout.height != expected_h {
                return Err(SamplingError::InvalidDimensions);
            }

            let tight_pitch = layout
                .width
                .checked_mul(RGBA8_TEXEL_BYTES)
                .ok_or(SamplingError::InvalidDimensions)?;
            if layout.row_pitch < tight_pitch {
                return Err(SamplingError::InvalidRowPitch);
            }

            let level_size = layout
                .row_pitch
                .checked_mul(layout.height)
                .ok_or(SamplingError::InvalidDimensions)?;
            let level_end = layout
                .offset
                .checked_add(level_size)
                .ok_or(SamplingError::InvalidOffset)?;

            total_end = total_end.max(level_end);
        }

        Ok(total_end)
    }
}

/// Calculate maximum allowable mip levels for a given width and height.
///
/// For arbitrary positive dimensions, the maximum mip count is:
/// `floor(log2(max(width, height))) + 1`
pub fn max_mip_levels(width: usize, height: usize) -> Result<u32, SamplingError> {
    if width == 0 || height == 0 {
        return Err(SamplingError::InvalidDimensions);
    }
    let max_dim = width.max(height);
    let levels = usize::BITS - max_dim.leading_zeros();
    Ok(levels)
}

/// Sample an RGBA8 texel deterministically from raw texture bytes using nearest-neighbor
/// filtering, clamp-to-edge addressing and explicit mip selection.
pub fn sample_rgba8_2d(
    data: &[u8],
    desc: &Rgba8TextureDesc,
    mip: u32,
    u: f32,
    v: f32,
) -> Result<[u8; 4], SamplingError> {
    if !u.is_finite() || !v.is_finite() {
        return Err(SamplingError::NonFiniteCoordinates);
    }

    let required = desc.required_size()?;
    if data.len() < required {
        return Err(SamplingError::BufferTooSmall);
    }

    if mip >= desc.mips {
        return Err(SamplingError::MipOutOfBounds);
    }

    let layout = desc
        .mip_layouts
        .get(mip as usize)
        .ok_or(SamplingError::MipOutOfBounds)?;

    // Clamp to edge [0.0, 1.0]
    let u_clamped = u.clamp(0.0, 1.0);
    let v_clamped = v.clamp(0.0, 1.0);

    // Nearest-neighbor discrete texel coordinates:
    // floor(coord * dim) clamped to [0, dim - 1]
    let tex_x = ((u_clamped as f64 * layout.width as f64).floor() as usize).min(layout.width - 1);
    let tex_y = ((v_clamped as f64 * layout.height as f64).floor() as usize).min(layout.height - 1);

    let row_start = layout
        .offset
        .checked_add(
            tex_y
                .checked_mul(layout.row_pitch)
                .ok_or(SamplingError::InvalidOffset)?,
        )
        .ok_or(SamplingError::InvalidOffset)?;

    let texel_offset = row_start
        .checked_add(
            tex_x
                .checked_mul(RGBA8_TEXEL_BYTES)
                .ok_or(SamplingError::InvalidOffset)?,
        )
        .ok_or(SamplingError::InvalidOffset)?;

    let bytes = data
        .get(texel_offset..texel_offset + RGBA8_TEXEL_BYTES)
        .ok_or(SamplingError::BufferTooSmall)?;

    Ok([bytes[0], bytes[1], bytes[2], bytes[3]])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_four_distinct_texels_in_2x2() {
        // 2x2 RGBA8 image:
        // Top-left: [255, 0, 0, 255] (Red)
        // Top-right: [0, 255, 0, 255] (Green)
        // Bottom-left: [0, 0, 255, 255] (Blue)
        // Bottom-right: [255, 255, 0, 255] (Yellow)
        let pixels: [u8; 16] = [
            255, 0, 0, 255, // (0, 0)
            0, 255, 0, 255, // (1, 0)
            0, 0, 255, 255, // (0, 1)
            255, 255, 0, 255, // (1, 1)
        ];
        let desc = Rgba8TextureDesc::new_packed(2, 2, 1).unwrap();

        // Sample centers of each texel:
        assert_eq!(
            sample_rgba8_2d(&pixels, &desc, 0, 0.25, 0.25).unwrap(),
            [255, 0, 0, 255]
        );
        assert_eq!(
            sample_rgba8_2d(&pixels, &desc, 0, 0.75, 0.25).unwrap(),
            [0, 255, 0, 255]
        );
        assert_eq!(
            sample_rgba8_2d(&pixels, &desc, 0, 0.25, 0.75).unwrap(),
            [0, 0, 255, 255]
        );
        assert_eq!(
            sample_rgba8_2d(&pixels, &desc, 0, 0.75, 0.75).unwrap(),
            [255, 255, 0, 255]
        );
    }

    #[test]
    fn test_normalized_uv_sampling() {
        let pixels: [u8; 16] = [
            10, 10, 10, 255, // (0, 0)
            20, 20, 20, 255, // (1, 0)
            30, 30, 30, 255, // (0, 1)
            40, 40, 40, 255, // (1, 1)
        ];
        let desc = Rgba8TextureDesc::new_packed(2, 2, 1).unwrap();

        // Within (0, 0) region [0.0, 0.5)
        assert_eq!(
            sample_rgba8_2d(&pixels, &desc, 0, 0.1, 0.1).unwrap(),
            [10, 10, 10, 255]
        );
        // Within (1, 1) region [0.5, 1.0]
        assert_eq!(
            sample_rgba8_2d(&pixels, &desc, 0, 0.9, 0.9).unwrap(),
            [40, 40, 40, 255]
        );
    }

    #[test]
    fn test_clamp_to_edge() {
        let pixels: [u8; 16] = [
            1, 1, 1, 255, 2, 2, 2, 255, // row 0
            3, 3, 3, 255, 4, 4, 4, 255, // row 1
        ];
        let desc = Rgba8TextureDesc::new_packed(2, 2, 1).unwrap();

        // Negative coordinates clamp to 0.0 -> (0, 0)
        assert_eq!(
            sample_rgba8_2d(&pixels, &desc, 0, -5.0, -10.0).unwrap(),
            [1, 1, 1, 255]
        );
        // Coordinates > 1.0 clamp to 1.0 -> (1, 1)
        assert_eq!(
            sample_rgba8_2d(&pixels, &desc, 0, 10.0, 50.0).unwrap(),
            [4, 4, 4, 255]
        );
        // Mixed clamping
        assert_eq!(
            sample_rgba8_2d(&pixels, &desc, 0, -1.0, 2.0).unwrap(),
            [3, 3, 3, 255]
        );
    }

    #[test]
    fn test_boundary_coordinates() {
        let pixels: [u8; 16] = [
            11, 11, 11, 255, 22, 22, 22, 255, 33, 33, 33, 255, 44, 44, 44, 255,
        ];
        let desc = Rgba8TextureDesc::new_packed(2, 2, 1).unwrap();

        // Exactly 0.0 -> texel 0
        assert_eq!(
            sample_rgba8_2d(&pixels, &desc, 0, 0.0, 0.0).unwrap(),
            [11, 11, 11, 255]
        );
        // Exactly 0.5 -> 0.5 * 2 = 1.0 -> texel 1
        assert_eq!(
            sample_rgba8_2d(&pixels, &desc, 0, 0.5, 0.0).unwrap(),
            [22, 22, 22, 255]
        );
        assert_eq!(
            sample_rgba8_2d(&pixels, &desc, 0, 0.0, 0.5).unwrap(),
            [33, 33, 33, 255]
        );
        // Exactly 1.0 -> clamped to texel 1
        assert_eq!(
            sample_rgba8_2d(&pixels, &desc, 0, 1.0, 1.0).unwrap(),
            [44, 44, 44, 255]
        );
    }

    #[test]
    fn test_multiple_mip_levels() {
        // Mip 0: 2x2 (16 bytes), Mip 1: 1x1 (4 bytes)
        let mut data = vec![0u8; 20];
        data[0..16].copy_from_slice(&[1, 0, 0, 255, 2, 0, 0, 255, 3, 0, 0, 255, 4, 0, 0, 255]);
        data[16..20].copy_from_slice(&[99, 99, 99, 255]);

        let desc = Rgba8TextureDesc::new_packed(2, 2, 2).unwrap();
        assert_eq!(desc.mip_layouts.len(), 2);
        assert_eq!(desc.mip_layouts[0].offset, 0);
        assert_eq!(desc.mip_layouts[1].offset, 16);

        // Mip 0 samples the 2x2 texels
        assert_eq!(
            sample_rgba8_2d(&data, &desc, 0, 0.75, 0.75).unwrap(),
            [4, 0, 0, 255]
        );
        // Mip 1 samples the single 1x1 texel across all UVs
        assert_eq!(
            sample_rgba8_2d(&data, &desc, 1, 0.1, 0.1).unwrap(),
            [99, 99, 99, 255]
        );
        assert_eq!(
            sample_rgba8_2d(&data, &desc, 1, 0.9, 0.9).unwrap(),
            [99, 99, 99, 255]
        );
    }

    #[test]
    fn test_invalid_dimensions() {
        assert_eq!(
            Rgba8TextureDesc::new_packed(0, 4, 1).unwrap_err(),
            SamplingError::InvalidDimensions
        );
        assert_eq!(
            Rgba8TextureDesc::new_packed(4, 0, 1).unwrap_err(),
            SamplingError::InvalidDimensions
        );
        assert_eq!(
            Rgba8TextureDesc::new_packed(4, 4, 0).unwrap_err(),
            SamplingError::InvalidMipCount
        );
        // 4x4 has at most 3 mips (4x4, 2x2, 1x1)
        assert_eq!(
            Rgba8TextureDesc::new_packed(4, 4, 4).unwrap_err(),
            SamplingError::InvalidMipCount
        );
    }

    #[test]
    fn test_non_power_of_two_max_mips() {
        // For 3x3, max(3, 3) = 3. floor(log2(3)) + 1 = 1 + 1 = 2 mips (3x3 and 1x1).
        assert_eq!(max_mip_levels(3, 3).unwrap(), 2);
        // Valid 2-mip descriptor
        let desc = Rgba8TextureDesc::new_packed(3, 3, 2).unwrap();
        assert_eq!(desc.mip_layouts.len(), 2);
        assert_eq!(desc.mip_layouts[0].width, 3);
        assert_eq!(desc.mip_layouts[0].height, 3);
        assert_eq!(desc.mip_layouts[1].width, 1);
        assert_eq!(desc.mip_layouts[1].height, 1);
        // 3 mips is invalid for 3x3
        assert_eq!(
            Rgba8TextureDesc::new_packed(3, 3, 3).unwrap_err(),
            SamplingError::InvalidMipCount
        );
    }

    #[test]
    fn test_invalid_pitches() {
        let desc = Rgba8TextureDesc {
            width: 4,
            height: 4,
            mips: 1,
            mip_layouts: vec![MipLayout {
                width: 4,
                height: 4,
                offset: 0,
                row_pitch: 12, // Tight pitch is 4 * 4 = 16, so 12 is invalid!
            }],
        };
        let dummy = vec![0u8; 64];
        assert_eq!(
            sample_rgba8_2d(&dummy, &desc, 0, 0.5, 0.5).unwrap_err(),
            SamplingError::InvalidRowPitch
        );
    }

    #[test]
    fn test_truncated_data() {
        let desc = Rgba8TextureDesc::new_packed(2, 2, 1).unwrap();
        // 2x2 requires 16 bytes; provide only 15
        let short_data = vec![0u8; 15];
        assert_eq!(
            sample_rgba8_2d(&short_data, &desc, 0, 0.0, 0.0).unwrap_err(),
            SamplingError::BufferTooSmall
        );
    }

    #[test]
    fn test_invalid_mip_indices() {
        let data = vec![0u8; 16];
        let desc = Rgba8TextureDesc::new_packed(2, 2, 1).unwrap();
        assert_eq!(
            sample_rgba8_2d(&data, &desc, 1, 0.0, 0.0).unwrap_err(),
            SamplingError::MipOutOfBounds
        );
        assert_eq!(
            sample_rgba8_2d(&data, &desc, 5, 0.0, 0.0).unwrap_err(),
            SamplingError::MipOutOfBounds
        );
    }

    #[test]
    fn test_non_finite_coordinates() {
        let data = vec![0u8; 16];
        let desc = Rgba8TextureDesc::new_packed(2, 2, 1).unwrap();

        assert_eq!(
            sample_rgba8_2d(&data, &desc, 0, f32::NAN, 0.0).unwrap_err(),
            SamplingError::NonFiniteCoordinates
        );
        assert_eq!(
            sample_rgba8_2d(&data, &desc, 0, 0.0, f32::NAN).unwrap_err(),
            SamplingError::NonFiniteCoordinates
        );
        assert_eq!(
            sample_rgba8_2d(&data, &desc, 0, f32::INFINITY, 0.5).unwrap_err(),
            SamplingError::NonFiniteCoordinates
        );
        assert_eq!(
            sample_rgba8_2d(&data, &desc, 0, 0.5, f32::NEG_INFINITY).unwrap_err(),
            SamplingError::NonFiniteCoordinates
        );
    }
}
