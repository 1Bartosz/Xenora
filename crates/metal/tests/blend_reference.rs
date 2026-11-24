// SPDX-License-Identifier: GPL-3.0-or-later

//! CPU check: the blend reference agrees with the independently derived
//! expectations for every blend case, colour and rounding flags alike. Runs
//! without a GPU.

#[path = "support/blend_cases.rs"]
mod blend_cases;

use blend_cases::{CLEAR, SIZE, cases, expected};
use xenora_ref::blend::{RefBlendDraw, render_blend};

#[test]
fn reference_matches_independent_expectations() {
    let (cases, expected) = (cases(), expected());
    assert_eq!(cases.len(), 38);
    assert_eq!(expected.len(), cases.len());
    for (case, want) in cases.iter().zip(&expected) {
        assert_eq!(case.name, want.name);
        let draws: Vec<_> = case
            .draws
            .iter()
            .map(|draw| RefBlendDraw {
                positions: draw.positions,
                color: draw.color,
                blend: draw.blend,
                mask: draw.mask,
            })
            .collect();
        let pixels = render_blend([SIZE, SIZE], CLEAR, &draws).expect("reference render");
        let color: Vec<_> = pixels.iter().map(|p| p.map(|c| c.value)).collect();
        let exact: Vec<_> = pixels.iter().map(|p| p.map(|c| c.exact)).collect();
        assert_eq!(color, want.color, "{} colour", case.name);
        assert_eq!(exact, want.exact, "{} rounding flags", case.name);
    }
}
