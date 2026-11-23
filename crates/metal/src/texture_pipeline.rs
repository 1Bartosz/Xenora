// SPDX-License-Identifier: GPL-3.0-or-later

//! Native Apple Metal RGBA8 texture upload and pipeline binding foundation.
//!
//! Provides validated RGBA8 2D texture allocation, mip upload, and binding definitions
//! for compute kernels and future translated pixel shader stages.

use std::ffi::c_void;
use std::ptr::NonNull;

use objc2::rc::{Retained, autoreleasepool};
use objc2::runtime::ProtocolObject;
use objc2_metal::{
    MTLDevice, MTLOrigin, MTLPixelFormat, MTLRegion, MTLResourceOptions, MTLSize, MTLStorageMode,
    MTLTexture, MTLTextureDescriptor, MTLTextureUsage,
};
use xenora_texture::{SamplerError, TextureMetadata};

use crate::MetalError;
use crate::sampler::MetalSampler;

/// Bytes per RGBA8 texel.
pub const RGBA8_TEXEL_BYTES: usize = 4;

/// Upper limit on total pixels per texture to prevent bounded resource overflow.
pub const MAX_TEXTURE_PIXELS: usize = 16384 * 16384;

/// Subresource mip level data descriptor for RGBA8 upload.
#[derive(Debug, Clone, Copy)]
pub struct Rgba8MipLevel<'a> {
    pub level: u32,
    pub width: usize,
    pub height: usize,
    pub row_pitch: usize,
    pub bytes: &'a [u8],
}

impl<'a> Rgba8MipLevel<'a> {
    /// Create a tightly packed mip level descriptor.
    pub fn new_packed(level: u32, width: usize, height: usize, bytes: &'a [u8]) -> Self {
        let row_pitch = width * RGBA8_TEXEL_BYTES;
        Self {
            level,
            width,
            height,
            row_pitch,
            bytes,
        }
    }
}

/// Description of an RGBA8 2D texture to allocate on Metal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgba8TextureDesc {
    pub width: usize,
    pub height: usize,
    pub mips: u32,
}

impl Rgba8TextureDesc {
    pub fn new(width: usize, height: usize, mips: u32) -> Result<Self, MetalError> {
        let meta = TextureMetadata::new(width, height, mips).map_err(|err: SamplerError| {
            MetalError::new("texture metadata validation", err.to_string())
        })?;

        let total_pixels = width
            .checked_mul(height)
            .ok_or_else(|| MetalError::new("texture dimensions", "pixel count overflow"))?;

        if total_pixels > MAX_TEXTURE_PIXELS {
            return Err(MetalError::new(
                "texture bounds",
                "texture exceeds maximum allowable pixels",
            ));
        }

        Ok(Self {
            width: meta.width,
            height: meta.height,
            mips: meta.mips,
        })
    }

    /// Compute expected dimensions for a given mip level.
    pub fn mip_extent(&self, level: u32) -> Result<(usize, usize), MetalError> {
        if level >= self.mips {
            return Err(MetalError::new(
                "mip level bounds",
                "mip level index is out of bounds",
            ));
        }
        let mip_w = (self.width >> level).max(1);
        let mip_h = (self.height >> level).max(1);
        Ok((mip_w, mip_h))
    }
}

/// Safe wrapper owning a native Metal RGBA8 2D texture.
#[derive(Debug, Clone)]
pub struct MetalRgba8Texture {
    texture: Retained<ProtocolObject<dyn MTLTexture>>,
    desc: Rgba8TextureDesc,
}

impl MetalRgba8Texture {
    /// Allocate an empty native Metal RGBA8 2D texture with storage mode Shared and ShaderRead usage.
    pub fn allocate(
        device: &ProtocolObject<dyn MTLDevice>,
        desc: Rgba8TextureDesc,
    ) -> Result<Self, MetalError> {
        autoreleasepool(|_| {
            // SAFETY: Validated non-zero dimensions within MAX_TEXTURE_PIXELS and RGBA8Unorm format.
            let descriptor = unsafe {
                MTLTextureDescriptor::texture2DDescriptorWithPixelFormat_width_height_mipmapped(
                    MTLPixelFormat::RGBA8Unorm,
                    desc.width,
                    desc.height,
                    desc.mips > 1,
                )
            };
            if desc.mips > 1 {
                // SAFETY: desc.mips is validated by TextureMetadata to not exceed max_mip_levels.
                unsafe {
                    descriptor.setMipmapLevelCount(desc.mips as usize);
                }
            }
            descriptor.setStorageMode(MTLStorageMode::Shared);
            descriptor.setResourceOptions(MTLResourceOptions::StorageModeShared);
            descriptor.setUsage(MTLTextureUsage::ShaderRead);

            let texture = device
                .newTextureWithDescriptor(&descriptor)
                .ok_or_else(|| {
                    MetalError::new(
                        "newTextureWithDescriptor",
                        "failed to allocate RGBA8 Metal texture",
                    )
                })?;

            Ok(Self { texture, desc })
        })
    }

    /// Upload a single validated mip level into this texture.
    pub fn upload_mip(&self, mip: &Rgba8MipLevel<'_>) -> Result<(), MetalError> {
        let (expected_w, expected_h) = self.desc.mip_extent(mip.level)?;
        if mip.width != expected_w || mip.height != expected_h {
            return Err(MetalError::new(
                "mip dimensions",
                format!(
                    "mip level {} expected {}x{}, got {}x{}",
                    mip.level, expected_w, expected_h, mip.width, mip.height
                ),
            ));
        }

        let tight_pitch = mip
            .width
            .checked_mul(RGBA8_TEXEL_BYTES)
            .ok_or_else(|| MetalError::new("row pitch", "row pitch overflow"))?;
        if mip.row_pitch < tight_pitch {
            return Err(MetalError::new(
                "row pitch",
                format!(
                    "row pitch {} smaller than tight pitch {}",
                    mip.row_pitch, tight_pitch
                ),
            ));
        }

        let min_bytes = mip
            .row_pitch
            .checked_mul(mip.height)
            .ok_or_else(|| MetalError::new("mip bytes", "byte length calculation overflow"))?;
        if mip.bytes.len() < min_bytes {
            return Err(MetalError::new(
                "mip buffer bounds",
                format!(
                    "buffer length {} is less than required {}",
                    mip.bytes.len(),
                    min_bytes
                ),
            ));
        }

        autoreleasepool(|_| {
            let region = MTLRegion {
                origin: MTLOrigin { x: 0, y: 0, z: 0 },
                size: MTLSize {
                    width: mip.width,
                    height: mip.height,
                    depth: 1,
                },
            };

            let ptr = NonNull::new(mip.bytes.as_ptr() as *mut c_void).ok_or_else(|| {
                MetalError::new("pointer", "null data pointer for texture upload")
            })?;

            // SAFETY: The bounds check verifies that mip.bytes has at least
            // `mip.row_pitch * mip.height` bytes, the region size matches mip dimensions,
            // the mip level is within the texture's allocated mip count, and the pointer is valid.
            unsafe {
                self.texture
                    .replaceRegion_mipmapLevel_withBytes_bytesPerRow(
                        region,
                        mip.level as usize,
                        ptr,
                        mip.row_pitch,
                    );
            }

            Ok(())
        })
    }

    /// Allocate and populate an RGBA8 2D texture from a set of mip levels.
    pub fn upload(
        device: &ProtocolObject<dyn MTLDevice>,
        desc: Rgba8TextureDesc,
        mips: &[Rgba8MipLevel<'_>],
    ) -> Result<Self, MetalError> {
        let texture = Self::allocate(device, desc)?;
        for mip in mips {
            texture.upload_mip(mip)?;
        }
        Ok(texture)
    }

    /// Access the underlying native Metal texture protocol object.
    #[inline]
    pub fn raw(&self) -> &ProtocolObject<dyn MTLTexture> {
        &self.texture
    }

    /// Access the texture descriptor.
    #[inline]
    pub fn desc(&self) -> &Rgba8TextureDesc {
        &self.desc
    }
}

/// Resource binding indices and lifetime conventions for sampling pipelines.
///
/// In the Xenora fixture ABI and translated shader conventions:
/// - Metal textures are bound to `[[texture(slot)]]`.
/// - Metal samplers are bound to `[[sampler(slot)]]`.
///
/// Under future RDNA2-to-Metal shader translation:
/// - An image resource descriptor (`T#`) maps to a `texture2d` binding.
/// - A sampler descriptor (`S#`) maps to a `sampler` binding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BindingSlot {
    pub index: u32,
}

impl BindingSlot {
    pub const fn new(index: u32) -> Self {
        Self { index }
    }
}

/// A validated texture-sampler pair ready for pipeline binding.
///
/// This struct holds strong references to the native Metal texture and sampler,
/// verifying that LOD ranges and explicit mip levels are consistent before
/// submitting draw or compute work.
#[derive(Debug, Clone)]
pub struct BoundSampledTexture {
    texture: MetalRgba8Texture,
    sampler: MetalSampler,
    texture_slot: BindingSlot,
    sampler_slot: BindingSlot,
}

impl BoundSampledTexture {
    /// Bind a texture and sampler together to specified shader slots.
    ///
    /// # Errors
    ///
    /// Returns [`MetalError`] if the sampler's explicit mip or LOD constraints
    /// are out of bounds for the texture's mip chain.
    pub fn bind(
        texture: MetalRgba8Texture,
        sampler: MetalSampler,
        texture_slot: BindingSlot,
        sampler_slot: BindingSlot,
    ) -> Result<Self, MetalError> {
        let meta = TextureMetadata::new(
            texture.desc().width,
            texture.desc().height,
            texture.desc().mips,
        )
        .map_err(|err: SamplerError| MetalError::new("texture metadata", err.to_string()))?;

        sampler
            .desc()
            .validate_against_texture(&meta)
            .map_err(|err: SamplerError| {
                MetalError::new("sampler texture compatibility", err.to_string())
            })?;

        Ok(Self {
            texture,
            sampler,
            texture_slot,
            sampler_slot,
        })
    }

    #[inline]
    pub fn texture(&self) -> &MetalRgba8Texture {
        &self.texture
    }

    #[inline]
    pub fn sampler(&self) -> &MetalSampler {
        &self.sampler
    }

    #[inline]
    pub fn texture_slot(&self) -> BindingSlot {
        self.texture_slot
    }

    #[inline]
    pub fn sampler_slot(&self) -> BindingSlot {
        self.sampler_slot
    }
}
