// SPDX-License-Identifier: GPL-3.0-or-later

//! Original depth-pass fixtures shared by the CPU reference check and the
//! native Metal test, and the parser for the independently derived
//! expectations in `fixtures/depth_expected.txt`.

use xenora_ref::depth::CompareFunction;

pub const SIZE: usize = 16;
pub const RED: [u8; 4] = [255, 0, 0, 255];
pub const GREEN: [u8; 4] = [0, 255, 0, 255];
pub const BLUE: [u8; 4] = [0, 0, 255, 255];
const BLACK: [u8; 4] = [0, 0, 0, 255];

/// Four original BC1 blocks (8x8 texels) using only endpoint and
/// transparent selectors, so every decoded texel is exact.
pub const BC1_BLOCKS: [u8; 32] = [
    0x00, 0xF8, 0x1F, 0x00, 0x44, 0x11, 0x50, 0x05, // red / blue, four-colour
    0xE0, 0x07, 0xFF, 0xFF, 0x0F, 0xF0, 0x4C, 0x31, // green / white, three-colour
    0xE0, 0xFF, 0x00, 0x00, 0x00, 0x55, 0x00, 0x55, // yellow / black
    0x10, 0x84, 0x41, 0x08, 0x14, 0x41, 0x14, 0x41, // grey / near black
];

// Triangles in pixel coordinates; no pixel centre lies on any edge.
const FULL: [[f32; 2]; 3] = [[0.0, 16.0], [32.0, 16.0], [0.0, -16.0]];
const A: [[f32; 2]; 3] = [[0.0, 0.0], [16.0, 0.0], [0.0, 12.0]];
const B: [[f32; 2]; 3] = [[4.0, 3.0], [16.0, 4.0], [6.0, 16.0]];
const C: [[f32; 2]; 3] = [[0.0, 8.0], [10.0, 8.0], [0.0, 16.0]];

#[derive(Clone, Copy, Debug)]
pub enum Fill {
    Flat([u8; 4]),
    Texture,
}

#[derive(Clone, Copy, Debug)]
pub struct Draw {
    pub positions: [[f32; 4]; 3],
    pub fill: Fill,
    pub compare: CompareFunction,
    pub write: bool,
}

#[derive(Debug)]
pub struct Case {
    pub name: String,
    pub clear_color: [u8; 4],
    pub clear_depth: Option<f32>,
    pub draws: Vec<Draw>,
}

fn draw(tri: [[f32; 2]; 3], z: f32, fill: Fill, compare: CompareFunction, write: bool) -> Draw {
    Draw {
        positions: tri.map(|[x, y]| [x / 8.0 - 1.0, 1.0 - y / 8.0, z, 1.0]),
        fill,
        compare,
        write,
    }
}

fn case(name: &str, clear_color: [u8; 4], clear_depth: Option<f32>, draws: Vec<Draw>) -> Case {
    Case {
        name: name.to_owned(),
        clear_color,
        clear_depth,
        draws,
    }
}

/// Every case, each exercising one feature, in fixture order.
pub fn cases() -> Vec<Case> {
    use CompareFunction::{Always, Less, LessEqual};
    let red = Fill::Flat(RED);
    let green = Fill::Flat(GREEN);
    let blue = Fill::Flat(BLUE);
    let mut cases = vec![case("clear", [16, 32, 48, 255], Some(0.375), Vec::new())];
    for compare in CompareFunction::ALL {
        for z in [0.25, 0.5, 0.75] {
            cases.push(case(
                &format!("compare_{compare:?}_{z}"),
                BLACK,
                Some(1.0),
                vec![
                    draw(A, 0.5, red, Always, true),
                    draw(B, z, green, compare, true),
                ],
            ));
        }
    }
    for (name, write) in [("write_disabled", false), ("write_enabled", true)] {
        cases.push(case(
            name,
            BLACK,
            Some(1.0),
            vec![
                draw(A, 0.5, red, Always, true),
                draw(B, 0.25, green, Always, write),
                draw(C, 0.75, blue, LessEqual, true),
            ],
        ));
    }
    let painter = |depth| {
        case(
            if depth {
                "disabled_attachment"
            } else {
                "no_attachment"
            },
            BLACK,
            depth.then_some(1.0),
            vec![
                draw(A, 0.25, red, Always, false),
                draw(B, 0.75, green, Always, false),
            ],
        )
    };
    cases.push(painter(true));
    cases.push(painter(false));
    cases.push(case(
        "textured",
        BLACK,
        Some(1.0),
        vec![
            draw(FULL, 0.5, Fill::Texture, Less, true),
            draw(B, 0.75, green, Less, true),
            draw(A, 0.25, red, Less, true),
        ],
    ));
    cases
}

/// Independently derived colour and depth of one case.
#[derive(Debug, PartialEq)]
pub struct Expected {
    pub name: String,
    pub color: Vec<[u8; 4]>,
    pub depth: Option<Vec<f32>>,
}

pub fn expected() -> Vec<Expected> {
    let text = include_str!("../fixtures/depth_expected.txt");
    let mut out = Vec::new();
    let mut lines = text.lines().filter(|line| !line.trim().is_empty());
    while let Some(header) = lines.next() {
        let name = header.strip_prefix("case ").expect("case line").to_owned();
        let (mut palette, mut depthkey) = (Vec::new(), Vec::new());
        let (mut color, mut depth, mut no_depth) = (Vec::new(), Vec::new(), false);
        for line in lines.by_ref() {
            let mut words = line.split_whitespace();
            match words.next().expect("keyword") {
                "palette" => {
                    let key = words.next().unwrap().chars().next().unwrap();
                    let rgba: Vec<u8> = words.map(|w| w.parse().unwrap()).collect();
                    palette.push((key, <[u8; 4]>::try_from(rgba).unwrap()));
                }
                "depthkey" => {
                    let key = words.next().unwrap().chars().next().unwrap();
                    depthkey.push((key, words.next().unwrap().parse::<f32>().unwrap()));
                }
                "color" => color.extend(words.next().unwrap().chars().map(|c| {
                    palette
                        .iter()
                        .find(|(k, _)| *k == c)
                        .expect("palette key")
                        .1
                })),
                "depth" => match words.next().unwrap() {
                    "none" => no_depth = true,
                    row => depth.extend(
                        row.chars()
                            .map(|c| depthkey.iter().find(|(k, _)| *k == c).expect("depth key").1),
                    ),
                },
                "end" => break,
                other => panic!("unexpected fixture line {other}"),
            }
        }
        assert_eq!(color.len(), SIZE * SIZE, "{name} colour rows");
        assert!(no_depth || depth.len() == SIZE * SIZE, "{name} depth rows");
        out.push(Expected {
            name,
            color,
            depth: (!no_depth).then_some(depth),
        });
    }
    out
}
