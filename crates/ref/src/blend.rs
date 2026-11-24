// SPDX-License-Identifier: GPL-3.0-or-later

//! Independent CPU reference for blended, write-masked triangle draws into an
//! RGBA8 unorm target.
//!
//! Every value is kept exact: a unorm8 value `v` means `v / 255`, so a
//! weighted term `value * factor` is an integer count of `1 / 255²`. The
//! result is clamped to 0..=1 and rounded to the nearest unorm8 value. The
//! reference also reports, per channel, whether that rounding was needed, so
//! a native comparison can hold exact results to exact equality. Coverage
//! comes from [`crate::depth::render_depth`].

use std::fmt;

use crate::depth::{CompareFunction, DepthRefError, RefDraw, RefFill, render_depth};

/// A blend factor; the colour factors use the alpha channel's own value for alpha.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RefBlendFactor {
    Zero,
    One,
    SourceColor,
    OneMinusSourceColor,
    SourceAlpha,
    OneMinusSourceAlpha,
    DestinationColor,
    OneMinusDestinationColor,
    DestinationAlpha,
    OneMinusDestinationAlpha,
}

impl RefBlendFactor {
    pub const ALL: [Self; 10] = [
        Self::Zero,
        Self::One,
        Self::SourceColor,
        Self::OneMinusSourceColor,
        Self::SourceAlpha,
        Self::OneMinusSourceAlpha,
        Self::DestinationColor,
        Self::OneMinusDestinationColor,
        Self::DestinationAlpha,
        Self::OneMinusDestinationAlpha,
    ];

    /// The factor for `channel` in units of 1/255.
    fn value(self, source: [u8; 4], destination: [u8; 4], channel: usize) -> i64 {
        let (s, d) = (i64::from(source[channel]), i64::from(destination[channel]));
        let (sa, da) = (i64::from(source[3]), i64::from(destination[3]));
        match self {
            Self::Zero => 0,
            Self::One => 255,
            Self::SourceColor => s,
            Self::OneMinusSourceColor => 255 - s,
            Self::SourceAlpha => sa,
            Self::OneMinusSourceAlpha => 255 - sa,
            Self::DestinationColor => d,
            Self::OneMinusDestinationColor => 255 - d,
            Self::DestinationAlpha => da,
            Self::OneMinusDestinationAlpha => 255 - da,
        }
    }
}

/// A blend operation; `Min` and `Max` ignore the factors.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RefBlendOperation {
    Add,
    Subtract,
    ReverseSubtract,
    Min,
    Max,
}

impl RefBlendOperation {
    pub const ALL: [Self; 5] = [
        Self::Add,
        Self::Subtract,
        Self::ReverseSubtract,
        Self::Min,
        Self::Max,
    ];
}

/// Separate RGB and alpha blend equations.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RefBlend {
    pub source_rgb: RefBlendFactor,
    pub destination_rgb: RefBlendFactor,
    pub rgb_operation: RefBlendOperation,
    pub source_alpha: RefBlendFactor,
    pub destination_alpha: RefBlendFactor,
    pub alpha_operation: RefBlendOperation,
}

/// One flat-coloured triangle; `mask` lists the written channels R, G, B, A.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RefBlendDraw {
    pub positions: [[f32; 4]; 3],
    pub color: [u8; 4],
    pub blend: Option<RefBlend>,
    pub mask: [bool; 4],
}

/// A blended channel value and whether rounding was needed to reach it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RefChannel {
    pub value: u8,
    pub exact: bool,
}

/// Blend one channel (0 to 3: R, G, B, A) of `source` over `destination`.
///
/// # Panics
///
/// Panics when `channel` is above 3.
pub fn blend_channel(
    blend: RefBlend,
    source: [u8; 4],
    destination: [u8; 4],
    channel: usize,
) -> RefChannel {
    let (factors, operation) = if channel == 3 {
        (
            (blend.source_alpha, blend.destination_alpha),
            blend.alpha_operation,
        )
    } else {
        (
            (blend.source_rgb, blend.destination_rgb),
            blend.rgb_operation,
        )
    };
    let (s, d) = (i64::from(source[channel]), i64::from(destination[channel]));
    let weighted_s = s * factors.0.value(source, destination, channel);
    let weighted_d = d * factors.1.value(source, destination, channel);
    // In units of 1/255².
    let result = match operation {
        RefBlendOperation::Add => weighted_s + weighted_d,
        RefBlendOperation::Subtract => weighted_s - weighted_d,
        RefBlendOperation::ReverseSubtract => weighted_d - weighted_s,
        RefBlendOperation::Min => s.min(d) * 255,
        RefBlendOperation::Max => s.max(d) * 255,
    }
    .clamp(0, 255 * 255);
    RefChannel {
        // Round half up: floor(result / 255 + 1/2).
        value: u8::try_from((2 * result + 255) / 510).expect("clamped to 0..=255"),
        exact: result % 255 == 0,
    }
}

#[derive(Debug, Eq, PartialEq)]
pub enum BlendRefError {
    /// The extent is empty or holds more than 4096 pixels, the native limit.
    Extent,
    /// The triangle coverage could not be computed.
    Raster(DepthRefError),
}

impl fmt::Display for BlendRefError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}

impl std::error::Error for BlendRefError {}

/// Render `draws` in order over a target cleared to `clear_color`, returning
/// each pixel's channels and whether each needed rounding (a cleared or
/// replaced channel is exact).
///
/// # Errors
///
/// Returns an error for an extent that is empty or above 4096 pixels, or when a triangle's
/// coverage cannot be computed (see [`render_depth`]).
pub fn render_blend(
    extent: [usize; 2],
    clear_color: [u8; 4],
    draws: &[RefBlendDraw],
) -> Result<Vec<[RefChannel; 4]>, BlendRefError> {
    let exact = |value| RefChannel { value, exact: true };
    let pixels = extent[0]
        .checked_mul(extent[1])
        .filter(|&n| (1..=4096).contains(&n))
        .ok_or(BlendRefError::Extent)?;
    let mut target = vec![clear_color.map(exact); pixels];
    for draw in draws {
        // Coverage: a white triangle over black, without depth attachment.
        let (coverage, _) = render_depth(
            extent,
            [0; 4],
            None,
            &[RefDraw {
                positions: draw.positions,
                fill: RefFill::Flat([255; 4]),
                compare: CompareFunction::Always,
                write: false,
            }],
            None,
        )
        .map_err(BlendRefError::Raster)?;
        for (pixel, covered) in target.iter_mut().zip(coverage) {
            if covered != [255; 4] {
                continue;
            }
            let destination = pixel.map(|channel| channel.value);
            for (channel, slot) in pixel.iter_mut().enumerate() {
                if !draw.mask[channel] {
                    continue;
                }
                *slot = match draw.blend {
                    None => exact(draw.color[channel]),
                    Some(blend) => blend_channel(blend, draw.color, destination, channel),
                };
            }
        }
    }
    Ok(target)
}

#[cfg(test)]
mod tests {
    use super::*;
    use RefBlendFactor::{One, OneMinusSourceAlpha, SourceAlpha, Zero};
    use RefBlendOperation::{Add, Max, Min, ReverseSubtract, Subtract};

    fn blend(
        source: RefBlendFactor,
        destination: RefBlendFactor,
        operation: RefBlendOperation,
    ) -> RefBlend {
        RefBlend {
            source_rgb: source,
            destination_rgb: destination,
            rgb_operation: operation,
            source_alpha: source,
            destination_alpha: destination,
            alpha_operation: operation,
        }
    }

    #[test]
    fn bad_extents_are_rejected() {
        for extent in [[0, 4], [4, 0], [usize::MAX, 2], [usize::MAX, 1], [65, 64]] {
            assert_eq!(
                render_blend(extent, [0; 4], &[]),
                Err(BlendRefError::Extent)
            );
        }
    }

    #[test]
    fn channels_follow_the_blend_equation() {
        let (s, d) = ([60, 180, 240, 96], [200, 100, 50, 160]);
        let channel = |state, c| blend_channel(state, s, d, c);
        // 60 * 96 / 255 + 200 * 159 / 255 = 147.29...
        assert_eq!(
            channel(blend(SourceAlpha, OneMinusSourceAlpha, Add), 0),
            RefChannel {
                value: 147,
                exact: false
            }
        );
        assert_eq!(
            channel(blend(One, Zero, Add), 1),
            RefChannel {
                value: 180,
                exact: true
            }
        );
        // 240 + 50 clamps to 255; 60 - 200 clamps to 0.
        assert_eq!(
            channel(blend(One, One, Add), 2),
            RefChannel {
                value: 255,
                exact: true
            }
        );
        assert_eq!(
            channel(blend(One, One, Subtract), 0),
            RefChannel {
                value: 0,
                exact: true
            }
        );
        assert_eq!(
            channel(blend(One, One, ReverseSubtract), 0),
            RefChannel {
                value: 140,
                exact: true
            }
        );
        // Min and Max ignore the factors.
        assert_eq!(
            channel(blend(Zero, Zero, Min), 1),
            RefChannel {
                value: 100,
                exact: true
            }
        );
        assert_eq!(
            channel(blend(Zero, Zero, Max), 3),
            RefChannel {
                value: 160,
                exact: true
            }
        );
    }
}
