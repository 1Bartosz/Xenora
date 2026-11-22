use std::ptr::NonNull;

use objc2::rc::autoreleasepool;
use objc2::runtime::NSObjectProtocol;
use objc2::sel;
use objc2_foundation::NSString;
use objc2_metal::{
    MTLClearColor, MTLCommandBuffer, MTLCommandBufferStatus, MTLCommandEncoder, MTLCommandQueue,
    MTLCreateSystemDefaultDevice, MTLDevice, MTLLibrary, MTLLoadAction, MTLOrigin, MTLPixelFormat,
    MTLPrimitiveType, MTLRegion, MTLRenderCommandEncoder, MTLRenderPassDescriptor,
    MTLRenderPipelineDescriptor, MTLRenderPipelineState, MTLShaderValidation, MTLSize,
    MTLStorageMode, MTLStoreAction, MTLTexture, MTLTextureDescriptor, MTLTextureUsage, MTLViewport,
};

const WIDTH: usize = 8;
const HEIGHT: usize = 6;
const PIXEL_FORMAT: MTLPixelFormat = MTLPixelFormat::RGBA8Unorm;
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
    let pixels = autoreleasepool(|_| render_fixture());
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

fn render_fixture() -> [u8; WIDTH * HEIGHT * 4] {
    let device = MTLCreateSystemDefaultDevice().expect("native Metal device");
    let queue = device.newCommandQueue().expect("render command queue");
    let library = device
        .newLibraryWithSource_options_error(&NSString::from_str(RENDER_SOURCE), None)
        .expect("render MSL compilation");
    let vertex = library
        .newFunctionWithName(&NSString::from_str("cover_viewport"))
        .expect("vertex entry");
    let fragment = library
        .newFunctionWithName(&NSString::from_str("draw_pattern"))
        .expect("fragment entry");
    let descriptor = MTLRenderPipelineDescriptor::new();
    descriptor.setVertexFunction(Some(&vertex));
    descriptor.setFragmentFunction(Some(&fragment));

    // SAFETY: Slot zero is the color attachment used by this single-target pass.
    let pipeline_target = unsafe { descriptor.colorAttachments().objectAtIndexedSubscript(0) };
    pipeline_target.setPixelFormat(PIXEL_FORMAT);
    let pipeline = device
        .newRenderPipelineStateWithDescriptor_error(&descriptor)
        .expect("offscreen render pipeline");
    if std::env::var("MTL_SHADER_VALIDATION").as_deref() == Ok("1") {
        assert!(pipeline.respondsToSelector(sel!(shaderValidation)));
        // SAFETY: The selector exists and this new pipeline is private to this thread.
        assert_eq!(
            unsafe { pipeline.shaderValidation() },
            MTLShaderValidation::Enabled
        );
    }

    // SAFETY: The fixed nonzero dimensions and RGBA8 format are valid for a
    // 2D texture on this device. This test only uses level zero.
    let texture_descriptor = unsafe {
        MTLTextureDescriptor::texture2DDescriptorWithPixelFormat_width_height_mipmapped(
            PIXEL_FORMAT,
            WIDTH,
            HEIGHT,
            false,
        )
    };
    texture_descriptor.setStorageMode(MTLStorageMode::Shared);
    texture_descriptor.setUsage(MTLTextureUsage::RenderTarget);
    let texture = device
        .newTextureWithDescriptor(&texture_descriptor)
        .expect("shared offscreen texture");
    let pass = MTLRenderPassDescriptor::renderPassDescriptor();
    // SAFETY: Slot zero is the only color attachment used by this pass.
    let attachment = unsafe { pass.colorAttachments().objectAtIndexedSubscript(0) };
    attachment.setTexture(Some(&texture));
    attachment.setLoadAction(MTLLoadAction::Clear);
    attachment.setClearColor(MTLClearColor {
        red: 1.0,
        green: 1.0,
        blue: 0.0,
        alpha: 1.0,
    });
    attachment.setStoreAction(MTLStoreAction::Store);
    let command = queue.commandBuffer().expect("render command buffer");
    let encoder = command
        .renderCommandEncoderWithDescriptor(&pass)
        .expect("render command encoder");
    encoder.setRenderPipelineState(&pipeline);
    encoder.setViewport(MTLViewport {
        originX: 0.0,
        originY: 0.0,
        width: WIDTH as f64,
        height: HEIGHT as f64,
        znear: 0.0,
        zfar: 1.0,
    });
    // SAFETY: The vertex function supports exactly indices 0..3 and binds no
    // buffers. The render pass and pipeline agree on the only attachment.
    unsafe { encoder.drawPrimitives_vertexStart_vertexCount(MTLPrimitiveType::Triangle, 0, 3) };
    encoder.endEncoding();
    command.commit();
    command.waitUntilCompleted();
    assert_eq!(
        command.status(),
        MTLCommandBufferStatus::Completed,
        "{:?}",
        command.error()
    );

    let region = MTLRegion {
        origin: MTLOrigin { x: 0, y: 0, z: 0 },
        size: MTLSize {
            width: WIDTH,
            height: HEIGHT,
            depth: 1,
        },
    };
    let mut pixels = [0; WIDTH * HEIGHT * 4];
    // SAFETY: The GPU has finished writing level zero of this shared texture.
    // The region and row stride describe exactly the initialized array.
    unsafe {
        texture.getBytes_bytesPerRow_fromRegion_mipmapLevel(
            NonNull::from(&mut pixels).cast(),
            WIDTH * 4,
            region,
            0,
        );
    }
    pixels
}
