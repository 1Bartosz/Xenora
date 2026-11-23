// SPDX-License-Identifier: GPL-3.0-or-later
use std::ptr::NonNull;

use objc2::rc::autoreleasepool;
use objc2::runtime::NSObjectProtocol;
use objc2::sel;
use objc2_foundation::NSString;
use objc2_metal::{
    MTLClearColor, MTLCommandBuffer, MTLCommandBufferStatus, MTLCommandEncoder, MTLCommandQueue,
    MTLCreateSystemDefaultDevice, MTLDevice, MTLGPUFamily, MTLLibrary, MTLLoadAction, MTLOrigin,
    MTLPixelFormat, MTLPrimitiveType, MTLRegion, MTLRenderCommandEncoder, MTLRenderPassDescriptor,
    MTLRenderPipelineDescriptor, MTLRenderPipelineState, MTLShaderValidation, MTLSize,
    MTLStorageMode, MTLStoreAction, MTLTexture, MTLTextureDescriptor, MTLTextureUsage, MTLViewport,
};

const WIDTH: usize = 8;
const HEIGHT: usize = 8;
const PIXEL_FORMAT: MTLPixelFormat = MTLPixelFormat::RGBA8Unorm;
const RENDER_SOURCE: &str = r"
#include <metal_stdlib>
using namespace metal;

vertex float4 cover_viewport(uint vertex_id [[vertex_id]]) {
    float x = vertex_id == 1u ? 3.0f : -1.0f;
    float y = vertex_id == 2u ? 3.0f : -1.0f;
    return float4(x, y, 0.5f, 1.0f);
}

fragment float4 draw_pattern(float4 position [[position]],
                             constant uint &parameter [[buffer(0)]]) {
    return float4(uint(position.x), uint(position.y), parameter, 255u) / 255.0f;
}
";

pub(crate) fn render(parameter: u8) -> Result<[u8; WIDTH * HEIGHT * 4], String> {
    autoreleasepool(|_| render_frame(u32::from(parameter)))
}

fn render_frame(parameter: u32) -> Result<[u8; WIDTH * HEIGHT * 4], String> {
    let device = MTLCreateSystemDefaultDevice().ok_or("native Metal device")?;
    let apple7 = device.supportsFamily(MTLGPUFamily::Apple7);
    println!(
        "X4 device={} Apple7={apple7} Apple9={} Metal4={}",
        device.name(),
        device.supportsFamily(MTLGPUFamily::Apple9),
        device.supportsFamily(MTLGPUFamily::Metal4),
    );
    if !apple7 {
        return Err("Apple family 7 capabilities are required".into());
    }
    let queue = device.newCommandQueue().ok_or("render command queue")?;
    let library = device
        .newLibraryWithSource_options_error(&NSString::from_str(RENDER_SOURCE), None)
        .map_err(|error| format!("render MSL compilation: {error:?}"))?;
    let vertex = library
        .newFunctionWithName(&NSString::from_str("cover_viewport"))
        .ok_or("vertex entry")?;
    let fragment = library
        .newFunctionWithName(&NSString::from_str("draw_pattern"))
        .ok_or("fragment entry")?;
    let descriptor = MTLRenderPipelineDescriptor::new();
    descriptor.setVertexFunction(Some(&vertex));
    descriptor.setFragmentFunction(Some(&fragment));

    // SAFETY: Slot zero is the color attachment used by this single-target pass.
    let pipeline_target = unsafe { descriptor.colorAttachments().objectAtIndexedSubscript(0) };
    pipeline_target.setPixelFormat(PIXEL_FORMAT);
    let pipeline = device
        .newRenderPipelineStateWithDescriptor_error(&descriptor)
        .map_err(|error| format!("offscreen render pipeline: {error:?}"))?;
    if std::env::var("MTL_SHADER_VALIDATION").as_deref() == Ok("1") {
        // SAFETY: The selector is checked before invoking it.
        if !pipeline.respondsToSelector(sel!(shaderValidation))
            || unsafe { pipeline.shaderValidation() } != MTLShaderValidation::Enabled
        {
            return Err("shader validation was requested but is unavailable".into());
        }
        println!("X4 render pipeline shader validation enabled");
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
        .ok_or("shared offscreen texture")?;
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
    let command = queue.commandBuffer().ok_or("render command buffer")?;
    let encoder = command
        .renderCommandEncoderWithDescriptor(&pass)
        .ok_or("render command encoder")?;
    encoder.setRenderPipelineState(&pipeline);
    encoder.setViewport(MTLViewport {
        originX: 0.0,
        originY: 0.0,
        width: WIDTH as f64,
        height: HEIGHT as f64,
        znear: 0.0,
        zfar: 1.0,
    });
    // SAFETY: Metal copies the four bytes during this call. Fragment slot zero
    // expects a single uint, and the local parameter remains alive for the call.
    unsafe {
        encoder.setFragmentBytes_length_atIndex(
            NonNull::from(&parameter).cast(),
            size_of::<u32>(),
            0,
        );
    }
    // SAFETY: The vertex function supports exactly indices 0..3 and binds no
    // vertex buffers. The render pass and pipeline agree on the only attachment.
    unsafe { encoder.drawPrimitives_vertexStart_vertexCount(MTLPrimitiveType::Triangle, 0, 3) };
    encoder.endEncoding();
    command.commit();
    command.waitUntilCompleted();
    if command.status() != MTLCommandBufferStatus::Completed {
        return Err(format!(
            "render completion: {:?} {:?}",
            command.status(),
            command.error()
        ));
    }
    println!("X4 Metal command completed");

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
    Ok(pixels)
}
