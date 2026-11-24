use xenora_metal::{BlendDraw, BlendFactor, BlendOperation, BlendPass, BlendState, ColorWriteMask};
use xenora_ref::blend::{RefBlend, RefBlendFactor, RefBlendOperation};

use super::blend_cases::{CLEAR, Case, SIZE, cases, expected};
use super::{context, native};

/// Tolerance, fixed before any native measurement: a channel whose exact
/// result is a whole unorm8 value must match exactly; a channel that needs
/// rounding may differ by at most this much, because Metal does not document
/// how it rounds blended unorm values.
const ROUNDING_TOLERANCE: u8 = 1;

fn factor(factor: RefBlendFactor) -> BlendFactor {
    match factor {
        RefBlendFactor::Zero => BlendFactor::Zero,
        RefBlendFactor::One => BlendFactor::One,
        RefBlendFactor::SourceColor => BlendFactor::SourceColor,
        RefBlendFactor::OneMinusSourceColor => BlendFactor::OneMinusSourceColor,
        RefBlendFactor::SourceAlpha => BlendFactor::SourceAlpha,
        RefBlendFactor::OneMinusSourceAlpha => BlendFactor::OneMinusSourceAlpha,
        RefBlendFactor::DestinationColor => BlendFactor::DestinationColor,
        RefBlendFactor::OneMinusDestinationColor => BlendFactor::OneMinusDestinationColor,
        RefBlendFactor::DestinationAlpha => BlendFactor::DestinationAlpha,
        RefBlendFactor::OneMinusDestinationAlpha => BlendFactor::OneMinusDestinationAlpha,
    }
}

fn operation(operation: RefBlendOperation) -> BlendOperation {
    match operation {
        RefBlendOperation::Add => BlendOperation::Add,
        RefBlendOperation::Subtract => BlendOperation::Subtract,
        RefBlendOperation::ReverseSubtract => BlendOperation::ReverseSubtract,
        RefBlendOperation::Min => BlendOperation::Min,
        RefBlendOperation::Max => BlendOperation::Max,
    }
}

fn state(blend: RefBlend) -> BlendState {
    BlendState {
        source_rgb: factor(blend.source_rgb),
        destination_rgb: factor(blend.destination_rgb),
        rgb_operation: operation(blend.rgb_operation),
        source_alpha: factor(blend.source_alpha),
        destination_alpha: factor(blend.destination_alpha),
        alpha_operation: operation(blend.alpha_operation),
    }
}

fn metal_draws(case: &Case) -> Vec<BlendDraw> {
    case.draws
        .iter()
        .map(|draw| BlendDraw {
            positions: draw.positions,
            color: draw.color,
            blend: draw.blend.map(state),
            write_mask: ColorWriteMask {
                red: draw.mask[0],
                green: draw.mask[1],
                blue: draw.mask[2],
                alpha: draw.mask[3],
            },
        })
        .collect()
}

/// Each case's native colour must equal the independently derived image,
/// per channel: exactly where the exact result is a whole unorm8 value, and
/// within [`ROUNDING_TOLERANCE`] where it needs rounding.
#[test]
fn blend_known_answers() {
    let gpu = context();
    let (cases, expected) = (cases(), expected());
    assert_eq!(cases.len(), expected.len());
    let (mut rounded, mut off_by_one, mut largest) = (0usize, 0usize, 0u8);
    for (case, want) in cases.iter().zip(&expected) {
        assert_eq!(case.name, want.name);
        let draws = metal_draws(case);
        let color = native(gpu.render_blend(&BlendPass {
            extent: [SIZE, SIZE],
            clear_color: CLEAR,
            draws: &draws,
        }));
        assert_eq!(color.len(), SIZE * SIZE * 4);
        for (index, ((pixel, wanted), exact)) in color
            .as_chunks::<4>()
            .0
            .iter()
            .zip(&want.color)
            .zip(&want.exact)
            .enumerate()
        {
            let (x, y) = (index % SIZE, index / SIZE);
            for channel in 0..4 {
                let difference = pixel[channel].abs_diff(wanted[channel]);
                let allowed = if exact[channel] {
                    0
                } else {
                    ROUNDING_TOLERANCE
                };
                assert!(
                    difference <= allowed,
                    "{} at ({x}, {y}) channel {channel}: {pixel:?} vs {wanted:?} (allowed {allowed})",
                    case.name
                );
                if !exact[channel] {
                    rounded += 1;
                    off_by_one += usize::from(difference == 1);
                    largest = largest.max(difference);
                }
            }
        }
    }
    println!(
        "checked {} blend passes; {rounded} rounded channels, {off_by_one} off by one, largest difference {largest}",
        cases.len()
    );
}

#[test]
fn too_many_blend_draws_are_rejected() {
    let gpu = context();
    let draw = BlendDraw {
        positions: [
            [-1.0, -1.0, 0.5, 1.0],
            [3.0, -1.0, 0.5, 1.0],
            [-1.0, 3.0, 0.5, 1.0],
        ],
        color: [255; 4],
        blend: None,
        write_mask: ColorWriteMask::ALL,
    };
    let error = gpu
        .render_blend(&BlendPass {
            extent: [SIZE, SIZE],
            clear_color: CLEAR,
            draws: &[draw; 17],
        })
        .expect_err("17 draws must be rejected");
    assert!(error.to_string().contains("at most 16 draws"), "{error}");
    let mut bad = draw;
    bad.positions[1][0] = f32::NAN;
    let error = gpu
        .render_blend(&BlendPass {
            extent: [SIZE, SIZE],
            clear_color: CLEAR,
            draws: &[bad],
        })
        .expect_err("non-finite positions must be rejected");
    assert!(error.to_string().contains("finite"), "{error}");
}
