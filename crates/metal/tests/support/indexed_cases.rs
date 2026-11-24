// SPDX-License-Identifier: GPL-3.0-or-later

//! Original indexed-draw fixtures shared by the CPU reference check and the
//! native Metal test, and the parser for the independently derived
//! expectations in `fixtures/indexed_expected.txt`.

pub const SIZE: usize = 16;
const RED: [u8; 4] = [255, 0, 0, 255];
const GREEN: [u8; 4] = [0, 255, 0, 255];
const BLUE: [u8; 4] = [0, 0, 255, 255];
const BLACK: [u8; 4] = [0, 0, 0, 255];

/// The four original BC1 blocks of the depth fixtures (8x8 texels), using
/// only endpoint and transparent selectors so every decoded texel is exact.
pub const BC1_BLOCKS: [u8; 32] = [
    0x00, 0xF8, 0x1F, 0x00, 0x44, 0x11, 0x50, 0x05, // red / blue, four-colour
    0xE0, 0x07, 0xFF, 0xFF, 0x0F, 0xF0, 0x4C, 0x31, // green / white, three-colour
    0xE0, 0xFF, 0x00, 0x00, 0x00, 0x55, 0x00, 0x55, // yellow / black
    0x10, 0x84, 0x41, 0x08, 0x14, 0x41, 0x14, 0x41, // grey / near black
];

/// Vertex buffer in pixel coordinates; no pixel centre lies on any edge of
/// a triangle the cases draw.
const VERTICES: [[f32; 2]; 13] = [
    [0.0, 0.0],
    [16.0, 0.0],
    [0.0, 12.0],
    [4.0, 3.0],
    [16.0, 4.0],
    [6.0, 16.0],
    [0.0, 8.0],
    [10.0, 8.0],
    [0.0, 16.0],
    [0.0, 0.0],
    [16.0, 0.0],
    [16.0, 15.0],
    [0.0, 16.0],
];

/// The index buffer of the 16-bit cases.
pub const INDICES: [u16; 21] = [
    0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 9, 11, 12, 2, 1, 0, 10, 11, 12,
];

/// The index buffer of the 32-bit cases: [`INDICES`] and three indices that
/// need more than 16 bits.
pub fn wide_indices() -> Vec<u32> {
    INDICES
        .iter()
        .map(|&index| u32::from(index))
        .chain([65545, 65546, 65547])
        .collect()
}

/// Clip-space positions of [`VERTICES`] at depth 0.5.
pub fn vertices() -> Vec<[f32; 4]> {
    VERTICES
        .iter()
        .map(|[x, y]| [x / 8.0 - 1.0, 1.0 - y / 8.0, 0.5, 1.0])
        .collect()
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Fill {
    Flat([u8; 4]),
    Texture,
}

#[derive(Clone, Copy, Debug)]
pub struct Draw {
    pub start: usize,
    pub count: usize,
    pub base_vertex: i32,
    pub fill: Fill,
}

#[derive(Debug)]
pub struct Case {
    pub name: &'static str,
    pub wide: bool,
    pub clear_color: [u8; 4],
    pub draws: Vec<Draw>,
}

/// Every case, in fixture order.
pub fn cases() -> Vec<Case> {
    let draw = |start, count, base_vertex, fill| Draw {
        start,
        count,
        base_vertex,
        fill,
    };
    let (red, green, blue, texture) = (
        Fill::Flat(RED),
        Fill::Flat(GREEN),
        Fill::Flat(BLUE),
        Fill::Texture,
    );
    let case = |name, wide, draws| Case {
        name,
        wide,
        clear_color: BLACK,
        draws,
    };
    vec![
        Case {
            clear_color: [16, 32, 48, 255],
            ..case("clear", false, Vec::new())
        },
        case("u16_first", false, vec![draw(0, 3, 0, red)]),
        case("u32_first", true, vec![draw(0, 3, 0, red)]),
        case("u16_odd_start", false, vec![draw(3, 3, 0, green)]),
        case("u32_start", true, vec![draw(3, 3, 0, green)]),
        case("u16_base_positive", false, vec![draw(0, 3, 6, blue)]),
        case("u16_base_negative", false, vec![draw(18, 3, -4, blue)]),
        case("u32_base_negative", true, vec![draw(18, 3, -4, blue)]),
        case("reversed_winding", false, vec![draw(15, 3, 0, red)]),
        case(
            "two_triangles_textured",
            false,
            vec![draw(9, 6, 0, texture)],
        ),
        case("textured_base_vertex", false, vec![draw(0, 3, 9, texture)]),
        case(
            "painter_order",
            false,
            vec![
                draw(0, 3, 0, red),
                draw(3, 3, 0, green),
                draw(0, 3, 6, blue),
            ],
        ),
        case(
            "u32_mixed",
            true,
            vec![
                draw(9, 6, 0, texture),
                draw(18, 3, -1, green),
                draw(6, 3, -6, red),
            ],
        ),
        case("flat_two_triangles", false, vec![draw(9, 6, 0, green)]),
        case("textured_first", false, vec![draw(0, 3, 0, texture)]),
        case("u32_high_indices", true, vec![draw(21, 3, -65536, red)]),
    ]
}

/// One case of `indexed_expected.txt`: fetched vertices per draw and colours.
#[derive(Debug)]
pub struct Expected {
    pub name: String,
    pub fetch: Vec<Vec<usize>>,
    pub color: Vec<[u8; 4]>,
}

/// Parse the independently derived expectations.
pub fn expected() -> Vec<Expected> {
    let text = include_str!("../fixtures/indexed_expected.txt");
    let mut out = Vec::new();
    let mut lines = text.lines().filter(|line| !line.trim().is_empty());
    while let Some(header) = lines.next() {
        let name = header.strip_prefix("case ").expect("case line").to_owned();
        let (mut fetch, mut palette, mut color) = (Vec::new(), Vec::new(), Vec::new());
        for line in lines.by_ref() {
            let mut words = line.split_whitespace();
            match words.next().expect("keyword") {
                "fetch" => fetch.push(words.map(|word| word.parse().unwrap()).collect()),
                "palette" => {
                    let key = words.next().unwrap().chars().next().unwrap();
                    let rgba: Vec<u8> = words.map(|word| word.parse().unwrap()).collect();
                    palette.push((key, <[u8; 4]>::try_from(rgba).unwrap()));
                }
                "color" => color.extend(words.next().unwrap().chars().map(|c| {
                    palette
                        .iter()
                        .find(|(key, _)| *key == c)
                        .expect("palette key")
                        .1
                })),
                "end" => break,
                other => panic!("unexpected fixture line {other}"),
            }
        }
        assert_eq!(color.len(), SIZE * SIZE, "{name} colour rows");
        out.push(Expected { name, fetch, color });
    }
    out
}
