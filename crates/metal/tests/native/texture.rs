// SPDX-License-Identifier: GPL-3.0-or-later

use xenora_texture::{Bc1Upload, Bc1UploadDesc, TiledBc1};

use super::context;

#[test]
fn samples_selected_bc1_layer_and_mip() {
    // Each 9×5 layer has two tiles for mip zero and one for mip one.
    // Color words are BC1 endpoint zero except for the transparent block,
    // where the ascending endpoints and selector three give transparent black.
    let red = [0, 248, 0, 0, 0, 0, 0, 0];
    let green = [224, 7, 0, 0, 0, 0, 0, 0];
    let blue = [31, 0, 0, 0, 0, 0, 0, 0];
    let white = [255, 255, 0, 0, 0, 0, 0, 0];
    let clear = [0, 0, 255, 255, 255, 255, 255, 255];
    let yellow = [224, 255, 0, 0, 0, 0, 0, 0];
    let black = [0; 8];
    let blocks = [
        black, black, black, black, black, black, black, black, black, black, black, black, red,
        green, white, clear, blue, black, yellow, black, green, black, black, black,
    ];
    let texture = TiledBc1::new(blocks.as_flattened(), 9, 5, 2, 2).unwrap();
    let context = context();
    let pixels = context.read_tiled_bc1(&texture, 1, 0).unwrap();
    let upper = [
        0xff00_00ff,
        0xff00_00ff,
        0xff00_00ff,
        0xff00_00ff,
        0xff00_ff00,
        0xff00_ff00,
        0xff00_ff00,
        0xff00_ff00,
        0xffff_0000,
    ];
    let lower = [
        0xffff_ffff,
        0xffff_ffff,
        0xffff_ffff,
        0xffff_ffff,
        0,
        0,
        0,
        0,
        0xff00_ffff,
    ];
    let expected: Vec<_> = upper.repeat(4).into_iter().chain(lower).collect();
    assert_eq!(pixels, expected);
    assert_eq!(
        context.read_tiled_bc1(&texture, 1, 1).unwrap(),
        [0xff00_ff00; 8]
    );
    assert_eq!(
        context.read_tiled_bc1(&texture, 0, 1).unwrap(),
        [0xff00_0000; 8]
    );
    assert_eq!(
        context
            .read_tiled_bc1(&texture, 2, 0)
            .unwrap_err()
            .operation,
        "texture subresource"
    );
    assert_eq!(
        context
            .read_tiled_bc1(&texture, 0, 2)
            .unwrap_err()
            .operation,
        "texture subresource"
    );
    println!(
        "device={} checked 61 RGBA8 texels from three subresources",
        context.device_name()
    );
}

#[test]
fn samples_pitched_bc1_upload_across_mips_and_layers() {
    let red = [0, 248, 0, 0, 0, 0, 0, 0];
    let green = [224, 7, 0, 0, 0, 0, 0, 0];
    let blue = [31, 0, 0, 0, 0, 0, 0, 0];
    let white = [255, 255, 0, 0, 0, 0, 0, 0];
    let clear = [0, 0, 255, 255, 255, 255, 255, 255];
    // Seven physical blocks per subresource. A base level consumes the
    // first two blocks of both rows, and mip 1 consumes only block zero.
    let pad = red;
    let blocks = [
        [blue, blue, pad, blue, blue, pad, pad],
        [green, pad, pad, pad, pad, pad, pad],
        [red, green, pad, blue, white, pad, pad],
        [clear, pad, pad, pad, pad, pad, pad],
    ];
    let upload = Bc1Upload::new(
        blocks.as_flattened().as_flattened(),
        Bc1UploadDesc {
            width: 5,
            height: 5,
            mips: 2,
            layers: 2,
            format: 1,
            usage: 1,
            row_pitch: 24,
            slice_pitch: 56,
        },
    )
    .unwrap();
    let gpu = context();
    let upper = [
        0xff00_00ff,
        0xff00_00ff,
        0xff00_00ff,
        0xff00_00ff,
        0xff00_ff00,
    ];
    let bottom = [
        0xffff_0000,
        0xffff_0000,
        0xffff_0000,
        0xffff_0000,
        0xffff_ffff,
    ];
    let expected: Vec<_> = upper.repeat(4).into_iter().chain(bottom).collect();
    assert_eq!(gpu.read_bc1_upload(&upload, 1, 0).unwrap(), expected);
    assert_eq!(gpu.read_bc1_upload(&upload, 1, 1).unwrap(), [0; 4]);
    assert_eq!(
        gpu.read_bc1_upload(&upload, 0, 1).unwrap(),
        [0xff00_ff00; 4]
    );
    assert_eq!(
        gpu.read_bc1_upload(&upload, 2, 0).unwrap_err().operation,
        "texture subresource"
    );
    println!("checked 33 RGBA8 texels from pitched BC1 subresources");
}
