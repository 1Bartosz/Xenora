// SPDX-License-Identifier: GPL-3.0-or-later

//! CPU check: the reference index fetch and rasteriser agree with the
//! independently derived expectations for every indexed case. Runs without a GPU.

#[path = "support/indexed_cases.rs"]
mod indexed_cases;

use indexed_cases::{BC1_BLOCKS, Fill, INDICES, SIZE, cases, expected, vertices};
use xenora_ref::depth::{RefFill, RefTexture};
use xenora_ref::indexed::{RefIndexedDraw, RefIndices, fetch, render_indexed};

fn bc1_texels() -> Vec<[u8; 4]> {
    let mut texels = vec![[0; 4]; 64];
    for (block, bytes) in BC1_BLOCKS.as_chunks::<8>().0.iter().enumerate() {
        for (index, texel) in xenora_texture::decode_bc1(*bytes).iter().enumerate() {
            texels[((block / 2) * 4 + index / 4) * 8 + (block % 2) * 4 + index % 4] = *texel;
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
    let wide = INDICES.map(u32::from);
    let vertices = vertices();
    let (cases, expected) = (cases(), expected());
    assert_eq!(cases.len(), 13);
    assert_eq!(expected.len(), cases.len());
    for (case, want) in cases.iter().zip(&expected) {
        assert_eq!(case.name, want.name);
        let indices = if case.wide {
            RefIndices::U32(&wide)
        } else {
            RefIndices::U16(&INDICES)
        };
        let draws: Vec<_> = case
            .draws
            .iter()
            .map(|draw| RefIndexedDraw {
                start: draw.start,
                count: draw.count,
                base_vertex: draw.base_vertex,
                fill: match draw.fill {
                    Fill::Flat(rgba) => RefFill::Flat(rgba),
                    Fill::Texture => RefFill::Texture,
                },
            })
            .collect();
        let fetched: Vec<_> = draws
            .iter()
            .enumerate()
            .map(|(number, draw)| fetch(number, draw, indices, vertices.len()).expect("fetch"))
            .collect();
        assert_eq!(fetched, want.fetch, "{} fetch", case.name);
        let color = render_indexed(
            [SIZE, SIZE],
            case.clear_color,
            &vertices,
            indices,
            &draws,
            Some(texture),
        )
        .expect("reference render");
        assert_eq!(color, want.color, "{} colour", case.name);
    }
}
