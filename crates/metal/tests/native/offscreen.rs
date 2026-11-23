use super::context;

const WIDTH: usize = 8;
const HEIGHT: usize = 6;
const RENDER_SOURCE: &str = r"
#include <metal_stdlib>
using namespace metal;

vertex float4 cover_viewport(uint vertex_id [[vertex_id]]) {
    float x = vertex_id == 1u ? 3.0f : -1.0f;
    float y = vertex_id == 2u ? 3.0f : -1.0f;
    return float4(x, y, 0.5f, 1.0f);
}

fragment float4 draw_pattern(float4 position [[position]]) {
    uint x = uint(position.x);
    uint y = uint(position.y);
    if (x == y) {
        discard_fragment();
    }
    return float4(y < 3u, x >= 4u, ((x ^ y) & 1u) != 0u, 1.0f);
}
";

#[test]
fn offscreen_vertex_and_fragment() {
    let pixels = context()
        .render_rgba8(
            RENDER_SOURCE,
            ["cover_viewport", "draw_pattern"],
            &[[0.0; 4]; 3],
            &[[0.0; 4]; 3],
            [WIDTH, HEIGHT],
            [255, 255, 0, 255],
        )
        .expect("draw native offscreen fixture");
    assert_eq!(pixels.len(), WIDTH * HEIGHT * 4);
    for (index, pixel) in pixels.as_chunks::<4>().0.iter().enumerate() {
        let x = index % WIDTH;
        let y = index / WIDTH;
        let expected = if x == y {
            [255, 255, 0, 255]
        } else {
            [
                if y < 3 { 255 } else { 0 },
                if x >= 4 { 255 } else { 0 },
                if (x + y) % 2 == 1 { 255 } else { 0 },
                255,
            ]
        };
        assert_eq!(*pixel, expected, "pixel ({x}, {y})");
    }
    println!("checked {} RGBA8 pixels", WIDTH * HEIGHT);
}
