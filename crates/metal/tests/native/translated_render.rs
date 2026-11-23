// SPDX-License-Identifier: GPL-3.0-or-later

use xenora_shader::{Architecture, PIXEL_ENTRY_POINT, VERTEX_ENTRY_POINT, compile_graphics};

use super::context;

const ENTRIES: [&str; 2] = [VERTEX_ENTRY_POINT, PIXEL_ENTRY_POINT];
const POSITIONS: [[f32; 4]; 3] = [
    [-1.0, -1.0, 0.5, 1.0],
    [3.0, -1.0, 0.5, 1.0],
    [-1.0, 3.0, 0.5, 1.0],
];
const CLEAR: [u8; 4] = [19, 43, 93, 251];

#[test]
fn translated_constant_pixel_shader() {
    let compiled = compile_graphics(
        Architecture::Gfx1030,
        include_bytes!("../../../../fixtures/rdna2/x9-constant-vs.bin"),
        include_bytes!("../../../../fixtures/rdna2/x9-constant-ps.bin"),
    )
    .expect("compile constant graphics fixture");
    let gpu = context();
    // This triangle falls entirely past the right clipping plane.
    let offscreen = POSITIONS.map(|[_, y, z, w]| [2.0, y, z, w]);
    for (positions, pixel) in [(&POSITIONS, [255, 0, 0, 255]), (&offscreen, CLEAR)] {
        let actual = gpu
            .render_rgba8(
                &compiled.msl,
                ENTRIES,
                positions,
                &[[0.0; 4]; 3],
                [4, 3],
                CLEAR,
            )
            .expect("draw translated constant pixel shader");
        assert_eq!(actual, pixel.repeat(12));
    }
}

#[test]
fn translated_varying_pixel_shader() {
    let compiled = compile_graphics(
        Architecture::Gfx1030,
        include_bytes!("../../../../fixtures/rdna2/x9-varying-vs.bin"),
        include_bytes!("../../../../fixtures/rdna2/x9-varying-ps.bin"),
    )
    .expect("compile varying graphics fixture");
    // Double the second vertex's homogeneous coordinates to keep the
    // projected triangle fixed but vary its perspective interpolation weight.
    let mut positions = POSITIONS;
    positions[1] = positions[1].map(|component| 2.0 * component);
    let colors = [
        [0.0, 0.0, 0.0, 1.0],
        [128.0 / 255.0, 0.0, 64.0 / 255.0, 1.0],
        [0.0, 128.0 / 255.0, 64.0 / 255.0, 1.0],
    ];
    let gpu = context();
    let actual = gpu
        .render_rgba8(&compiled.msl, ENTRIES, &positions, &colors, [4, 4], CLEAR)
        .expect("draw translated varying pixel shader");

    // The triangle legs span eight pixels from the bottom-left viewport
    // corner. At pixel centers the second and third vertex's affine
    // weights are (2x+1)/16 and (7-2y)/16. Dividing each by its clip W
    // and normalizing gives the perspective weights below. The odd
    // denominator makes nearest-byte conversion unambiguous.
    let expected: Vec<u8> = (0..4)
        .flat_map(|y| {
            (0..4).flat_map(move |x| {
                let u = 2 * x + 1;
                let v = 2 * (7 - 2 * y);
                let sum = 32 - u;
                let byte = |value| ((value + sum / 2) / sum) as u8;
                [byte(128 * u), byte(128 * v), byte(64 * (u + v)), 255]
            })
        })
        .collect();
    assert_eq!(actual, expected);

    let swapped_colors = colors.map(|[r, g, b, a]| [g, r, b, a]);
    let swapped = gpu
        .render_rgba8(
            &compiled.msl,
            ENTRIES,
            &positions,
            &swapped_colors,
            [4, 4],
            CLEAR,
        )
        .expect("draw the same translated shader with red and green vertex data swapped");
    let mut expected_swapped = expected.clone();
    for pixel in expected_swapped.as_chunks_mut::<4>().0 {
        pixel.swap(0, 1);
    }
    assert_ne!(expected_swapped, expected);
    assert_eq!(swapped, expected_swapped);
    println!("checked 16 perspective-interpolated RGBA8 pixels per color input");
}

#[test]
fn translated_render_rejects_out_of_policy_inputs() {
    let gpu = context();
    for extent in [[0, 1], [64, 65], [usize::MAX, 2]] {
        let error = gpu
            .render_rgba8("unused", ENTRIES, &POSITIONS, &[[0.0; 4]; 3], extent, CLEAR)
            .expect_err("invalid dimensions must fail before MSL compilation");
        assert_eq!(error.operation, "render bounds");
    }
    let error = gpu
        .render_rgba8(
            "unused",
            ENTRIES,
            &POSITIONS,
            &[[f32::NAN; 4]; 3],
            [1, 1],
            CLEAR,
        )
        .expect_err("non-finite colors must fail before MSL compilation");
    assert_eq!(error.operation, "render input");
}

#[test]
fn transformed_vertex_shader_draws_three_interior_pixels() {
    let constant_ps = include_bytes!("../../../../fixtures/rdna2/x9-constant-ps.bin");
    let transformed = compile_graphics(
        Architecture::Gfx1030,
        include_bytes!("../../../../fixtures/rdna2/x9-transform-vs.bin"),
        constant_ps,
    )
    .expect("compile position transform");
    let passthrough = compile_graphics(
        Architecture::Gfx1030,
        include_bytes!("../../../../fixtures/rdna2/x9-constant-vs.bin"),
        constant_ps,
    )
    .expect("compile pass-through control");
    let positions = [
        [-1.5, -1.0, 0.5, 1.0],
        [0.5, -1.0, 0.5, 1.0],
        [-1.5, 2.0, 0.5, 1.0],
    ];

    // Scaling x and y by half and biasing x by one quarter places the
    // vertices at (1, 3), (3, 3), and (1, 0) in this 4x4 viewport.
    // A sample center is inside exactly when 1 < x < 3 and
    // 3*(x-1)/2 < y < 3. No edge crosses a sample center.
    let mut expected = [CLEAR; 16];
    for index in [5, 9, 10] {
        expected[index] = [255, 0, 0, 255];
    }
    let gpu = context();
    let colors = [[0.0; 4]; 3];
    let transformed_pixels = gpu
        .render_rgba8(
            &transformed.msl,
            ENTRIES,
            &positions,
            &colors,
            [4, 4],
            CLEAR,
        )
        .expect("draw translated transform");
    assert_eq!(transformed_pixels, expected.as_flattened());

    let passthrough_pixels = gpu
        .render_rgba8(
            &passthrough.msl,
            ENTRIES,
            &positions,
            &colors,
            [4, 4],
            CLEAR,
        )
        .expect("draw unchanged positions through pass-through control");
    // Without the transform, the diagonal extends from (-1, -2) to (3, 4)
    // and the viewport clips both the left and upper edges of the triangle.
    let mut expected_passthrough = [CLEAR; 16];
    for index in [0, 4, 8, 9, 12, 13, 14] {
        expected_passthrough[index] = [255, 0, 0, 255];
    }
    assert_eq!(passthrough_pixels, expected_passthrough.as_flattened());
    println!("checked 3 covered pixels and 13 clear pixels from translated vertex arithmetic");
}
