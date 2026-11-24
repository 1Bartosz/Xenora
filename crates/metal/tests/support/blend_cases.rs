// SPDX-License-Identifier: GPL-3.0-or-later

//! Original blend-pass fixtures shared by the CPU reference check and the
//! native Metal test, and the parser for the independently derived
//! expectations in `fixtures/blend_expected.txt`.
//!
//! The expectations record, per channel, whether the exact result needed
//! rounding; the native test's tolerance (`native/blend.rs`) depends on it.

use xenora_ref::blend::{RefBlend, RefBlendFactor, RefBlendOperation};

pub const SIZE: usize = 16;
pub const CLEAR: [u8; 4] = [32, 64, 96, 128];
const DST: [u8; 4] = [200, 100, 50, 160];
const SRC: [u8; 4] = [60, 180, 240, 96];

// Triangles in pixel coordinates; no pixel centre lies on any edge.
const A: [[f32; 2]; 3] = [[0.0, 0.0], [16.0, 0.0], [0.0, 12.0]];
const B: [[f32; 2]; 3] = [[4.0, 3.0], [16.0, 4.0], [6.0, 16.0]];

#[derive(Clone, Copy, Debug)]
pub struct Draw {
    pub positions: [[f32; 4]; 3],
    pub color: [u8; 4],
    pub blend: Option<RefBlend>,
    /// Written channels R, G, B, A.
    pub mask: [bool; 4],
}

#[derive(Debug)]
pub struct Case {
    pub name: String,
    pub draws: Vec<Draw>,
}

fn clip(tri: [[f32; 2]; 3]) -> [[f32; 4]; 3] {
    tri.map(|[x, y]| [x / 8.0 - 1.0, 1.0 - y / 8.0, 0.5, 1.0])
}

fn same(
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

/// Destination triangle A, then source triangle B with `blend` and `mask`.
fn case(name: String, blend: Option<RefBlend>, mask: [bool; 4]) -> Case {
    Case {
        name,
        draws: vec![
            Draw {
                positions: clip(A),
                color: DST,
                blend: None,
                mask: [true; 4],
            },
            Draw {
                positions: clip(B),
                color: SRC,
                blend,
                mask,
            },
        ],
    }
}

/// Every case, in fixture order.
pub fn cases() -> Vec<Case> {
    use RefBlendFactor::{One, OneMinusSourceAlpha, SourceAlpha, Zero};
    use RefBlendOperation::Add;
    let all = [true; 4];
    let mut cases = vec![
        Case {
            name: "clear".to_owned(),
            draws: Vec::new(),
        },
        case("disabled".to_owned(), None, all),
    ];
    for factor in RefBlendFactor::ALL {
        cases.push(case(
            format!("src_{factor:?}"),
            Some(same(factor, Zero, Add)),
            all,
        ));
    }
    for factor in RefBlendFactor::ALL {
        cases.push(case(
            format!("dst_{factor:?}"),
            Some(same(Zero, factor, Add)),
            all,
        ));
    }
    for operation in RefBlendOperation::ALL {
        cases.push(case(
            format!("op_{operation:?}"),
            Some(same(SourceAlpha, OneMinusSourceAlpha, operation)),
            all,
        ));
    }
    cases.push(case(
        "additive_clamp".to_owned(),
        Some(same(One, One, Add)),
        all,
    ));
    cases.push(case(
        "separate_alpha".to_owned(),
        Some(RefBlend {
            source_alpha: One,
            destination_alpha: Zero,
            ..same(SourceAlpha, OneMinusSourceAlpha, Add)
        }),
        all,
    ));
    for (name, mask) in [
        ("R", [true, false, false, false]),
        ("G", [false, true, false, false]),
        ("B", [false, false, true, false]),
        ("A", [false, false, false, true]),
        ("RGB", [true, true, true, false]),
        ("none", [false; 4]),
        ("RGBA", all),
    ] {
        cases.push(case(format!("mask_{name}"), None, mask));
    }
    cases.push(case(
        "mask_RG_blended".to_owned(),
        Some(same(SourceAlpha, OneMinusSourceAlpha, Add)),
        [true, true, false, false],
    ));
    let mut first = case(
        "first_draw_blended".to_owned(),
        None,
        [true, true, false, false],
    );
    first.draws[0].blend = Some(same(SourceAlpha, OneMinusSourceAlpha, Add));
    first.draws[0].mask = [false, true, true, true];
    cases.push(case(
        "separate_operation".to_owned(),
        Some(RefBlend {
            source_alpha: One,
            destination_alpha: One,
            alpha_operation: RefBlendOperation::ReverseSubtract,
            ..same(SourceAlpha, OneMinusSourceAlpha, Add)
        }),
        all,
    ));
    cases.push(first);
    cases
}

/// One case of `blend_expected.txt`: per pixel, the colour and whether each
/// channel needed rounding.
#[derive(Debug)]
pub struct Expected {
    pub name: String,
    pub color: Vec<[u8; 4]>,
    pub exact: Vec<[bool; 4]>,
}

/// Parse the independently derived expectations.
pub fn expected() -> Vec<Expected> {
    let text = include_str!("../fixtures/blend_expected.txt");
    let mut out = Vec::new();
    let mut lines = text.lines().filter(|line| !line.trim().is_empty());
    while let Some(header) = lines.next() {
        let name = header.strip_prefix("case ").expect("case line").to_owned();
        let (mut palette, mut color, mut exact) = (Vec::new(), Vec::new(), Vec::new());
        for line in lines.by_ref() {
            let words: Vec<&str> = line.split_whitespace().collect();
            match words[0] {
                "palette" => {
                    let key = words[1].chars().next().unwrap();
                    let rgba: Vec<u8> = words[2..6].iter().map(|w| w.parse().unwrap()).collect();
                    let flags: Vec<bool> = words[6].chars().map(|c| c == 'e').collect();
                    palette.push((
                        key,
                        <[u8; 4]>::try_from(rgba).unwrap(),
                        <[bool; 4]>::try_from(flags).unwrap(),
                    ));
                }
                "color" => {
                    for c in words[1].chars() {
                        let entry = palette.iter().find(|(k, ..)| *k == c).expect("palette key");
                        color.push(entry.1);
                        exact.push(entry.2);
                    }
                }
                "end" => break,
                other => panic!("unexpected fixture line {other}"),
            }
        }
        assert_eq!(color.len(), SIZE * SIZE, "{name} colour rows");
        out.push(Expected { name, color, exact });
    }
    out
}
