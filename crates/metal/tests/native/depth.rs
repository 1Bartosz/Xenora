use xenora_metal::{DepthCompare, DepthDraw, DepthFill, DepthPass, DepthState};
use xenora_ref::depth::CompareFunction;
use xenora_texture::{Bc1Upload, Bc1UploadDesc};

use super::depth_cases::{BC1_BLOCKS, Case, Fill, GREEN, SIZE, cases, expected};
use super::{context, native};

fn compare(function: CompareFunction) -> DepthCompare {
    match function {
        CompareFunction::Never => DepthCompare::Never,
        CompareFunction::Less => DepthCompare::Less,
        CompareFunction::Equal => DepthCompare::Equal,
        CompareFunction::LessEqual => DepthCompare::LessEqual,
        CompareFunction::Greater => DepthCompare::Greater,
        CompareFunction::NotEqual => DepthCompare::NotEqual,
        CompareFunction::GreaterEqual => DepthCompare::GreaterEqual,
        CompareFunction::Always => DepthCompare::Always,
    }
}

fn metal_draws(case: &Case) -> Vec<DepthDraw> {
    case.draws
        .iter()
        .map(|draw| DepthDraw {
            positions: draw.positions,
            fill: match draw.fill {
                Fill::Flat(rgba) => DepthFill::Flat(rgba),
                Fill::Texture => DepthFill::Bc1Texture,
            },
            depth: DepthState {
                compare: compare(draw.compare),
                write: draw.write,
            },
        })
        .collect()
}

fn bc1_upload() -> Bc1Upload<'static> {
    Bc1Upload::new(
        &BC1_BLOCKS,
        Bc1UploadDesc {
            width: 8,
            height: 8,
            mips: 1,
            layers: 1,
            format: 1,
            usage: 1,
            row_pitch: 16,
            slice_pitch: 32,
        },
    )
    .expect("valid BC1 upload")
}

/// Each case's native colour and depth must equal the independently derived
/// values (which the CPU reference also reproduces in `depth_reference`).
/// Colour and depth are compared separately, per pixel.
#[test]
fn depth_attachment_known_answers() {
    let gpu = context();
    let upload = bc1_upload();
    let expected = expected();
    let cases = cases();
    assert_eq!(cases.len(), expected.len());
    let mut checked = 0;
    for (case, want) in cases.iter().zip(&expected) {
        assert_eq!(case.name, want.name);
        let draws = metal_draws(case);
        let output = native(
            gpu.render_depth(&DepthPass {
                extent: [SIZE, SIZE],
                clear_color: case.clear_color,
                clear_depth: case.clear_depth,
                draws: &draws,
                // Only the textured case needs BC support, so a device without it
                // fails that case alone, as a capability error.
                texture: case
                    .draws
                    .iter()
                    .any(|draw| matches!(draw.fill, Fill::Texture))
                    .then_some(&upload),
            }),
        );
        for (index, (pixel, wanted)) in output
            .color
            .as_chunks::<4>()
            .0
            .iter()
            .zip(&want.color)
            .enumerate()
        {
            let (x, y) = (index % SIZE, index / SIZE);
            assert_eq!(pixel, wanted, "{} colour at ({x}, {y})", case.name);
        }
        assert_eq!(output.color.len(), SIZE * SIZE * 4);
        match (&output.depth, &want.depth) {
            (Some(depth), Some(wanted)) => {
                assert_eq!(depth.len(), wanted.len());
                for (index, (value, wanted)) in depth.iter().zip(wanted).enumerate() {
                    let (x, y) = (index % SIZE, index / SIZE);
                    assert_eq!(
                        value.to_bits(),
                        wanted.to_bits(),
                        "{} depth at ({x}, {y}): {value} vs {wanted}",
                        case.name
                    );
                }
            }
            (None, None) => {}
            (got, want) => panic!("{}: depth presence {got:?} vs {want:?}", case.name),
        }
        checked += 1;
    }
    println!("checked {checked} depth passes of {} pixels", SIZE * SIZE);
}

#[test]
fn depth_state_without_attachment_is_rejected() {
    let gpu = context();
    let draw = DepthDraw {
        positions: [
            [-1.0, -1.0, 0.5, 1.0],
            [3.0, -1.0, 0.5, 1.0],
            [-1.0, 3.0, 0.5, 1.0],
        ],
        fill: DepthFill::Flat(GREEN),
        depth: DepthState {
            compare: DepthCompare::Less,
            write: false,
        },
    };
    let pass = |clear_depth, draw: DepthDraw| {
        gpu.render_depth(&DepthPass {
            extent: [SIZE, SIZE],
            clear_color: [0; 4],
            clear_depth,
            draws: &[draw],
            texture: None,
        })
    };
    let error = pass(None, draw).expect_err("depth test without attachment");
    assert!(!error.is_capability_unavailable(), "{error}");
    for clear in [-0.25, 1.5, f32::NAN] {
        pass(Some(clear), draw).expect_err("clear depth outside 0..=1");
    }
    let textured = DepthDraw {
        fill: DepthFill::Bc1Texture,
        ..draw
    };
    pass(Some(1.0), textured).expect_err("textured draw without texture");
}
