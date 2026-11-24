// SPDX-License-Identifier: GPL-3.0-or-later

//! CPU check: the reference rasteriser agrees with the independently derived
//! expectations for every depth case. Runs without a GPU.

#[path = "support/depth_cases.rs"]
mod depth_cases;

use depth_cases::{BC1_BLOCKS, Fill, SIZE, cases, expected};
use xenora_ref::depth::{RefDraw, RefFill, RefTexture, render_depth};

/// Row-major 8x8 texels decoded block by block with the CPU BC1 decoder.
pub fn bc1_texels() -> Vec<[u8; 4]> {
    let mut texels = vec![[0; 4]; 64];
    for (block, bytes) in BC1_BLOCKS.as_chunks::<8>().0.iter().enumerate() {
        let decoded = xenora_texture::decode_bc1(*bytes);
        for (index, texel) in decoded.iter().enumerate() {
            let x = (block % 2) * 4 + index % 4;
            let y = (block / 2) * 4 + index / 4;
            texels[y * 8 + x] = *texel;
        }
    }
    texels
}

#[test]
fn reference_matches_independent_expectations() {
    let texels = bc1_texels();
    let texture = RefTexture {
        width: 8,
        height: 8,
        texels: &texels,
    };
    let expected = expected();
    let cases = cases();
    assert_eq!(cases.len(), 30);
    assert_eq!(expected.len(), cases.len());
    for (case, want) in cases.iter().zip(&expected) {
        assert_eq!(case.name, want.name);
        let draws: Vec<_> = case
            .draws
            .iter()
            .map(|draw| RefDraw {
                positions: draw.positions,
                fill: match draw.fill {
                    Fill::Flat(rgba) => RefFill::Flat(rgba),
                    Fill::Texture => RefFill::Texture,
                },
                compare: draw.compare,
                write: draw.write,
            })
            .collect();
        let (color, depth) = render_depth(
            [SIZE, SIZE],
            case.clear_color,
            case.clear_depth,
            &draws,
            Some(texture),
        )
        .expect("reference render");
        assert_eq!(color, want.color, "{} colour", case.name);
        assert_eq!(depth, want.depth, "{} depth", case.name);
    }
}
