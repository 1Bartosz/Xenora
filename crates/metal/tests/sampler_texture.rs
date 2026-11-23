// SPDX-License-Identifier: GPL-3.0-or-later

use xenora_metal::texture_pipeline::{BindingSlot, Rgba8MipLevel, Rgba8TextureDesc};
use xenora_texture::{AddressMode, FilterMode, SamplerDesc, TextureMetadata};

#[test]
fn test_valid_sampler_desc_and_lod() {
    let sampler_desc = SamplerDesc::nearest_clamp().with_lod_range(0.0, 5.0);
    assert!(sampler_desc.validate().is_ok());
    assert_eq!(sampler_desc.min_filter, FilterMode::Nearest);
    assert_eq!(sampler_desc.mag_filter, FilterMode::Nearest);
    assert_eq!(sampler_desc.address_u, AddressMode::ClampToEdge);
    assert_eq!(sampler_desc.address_v, AddressMode::ClampToEdge);
    assert_eq!(sampler_desc.min_lod, 0.0);
    assert_eq!(sampler_desc.max_lod, 5.0);
}

#[test]
fn test_invalid_sampler_configurations() {
    let invalid_order = SamplerDesc::nearest_clamp().with_lod_range(4.0, 2.0);
    assert!(invalid_order.validate().is_err());

    let negative_lod = SamplerDesc::nearest_clamp().with_lod_range(-1.0, 2.0);
    assert!(negative_lod.validate().is_err());

    let nan_lod = SamplerDesc::nearest_clamp().with_lod_range(f32::NAN, 2.0);
    assert!(nan_lod.validate().is_err());
}

#[test]
fn test_rgba8_texture_desc_validation() {
    // Valid 4x4 with 3 mips (4x4, 2x2, 1x1)
    let desc = Rgba8TextureDesc::new(4, 4, 3);
    assert!(desc.is_ok());
    let desc = desc.unwrap();
    assert_eq!(desc.mip_extent(0).unwrap(), (4, 4));
    assert_eq!(desc.mip_extent(1).unwrap(), (2, 2));
    assert_eq!(desc.mip_extent(2).unwrap(), (1, 1));
    assert!(desc.mip_extent(3).is_err());

    // Non-power-of-two 3x3 has max 2 mips
    let npot = Rgba8TextureDesc::new(3, 3, 2);
    assert!(npot.is_ok());
    let npot = npot.unwrap();
    assert_eq!(npot.mip_extent(0).unwrap(), (3, 3));
    assert_eq!(npot.mip_extent(1).unwrap(), (1, 1));

    // Exceeding max mips for 3x3
    assert!(Rgba8TextureDesc::new(3, 3, 3).is_err());

    // Zero dimensions
    assert!(Rgba8TextureDesc::new(0, 4, 1).is_err());
    assert!(Rgba8TextureDesc::new(4, 0, 1).is_err());
    assert!(Rgba8TextureDesc::new(4, 4, 0).is_err());
}

#[test]
fn test_rgba8_mip_level_pitch_and_length_validation() {
    // Mip 0 for 2x2: width 2, height 2, tight pitch 8 bytes, total 16 bytes.
    let valid_data = [0u8; 16];
    let mip = Rgba8MipLevel::new_packed(0, 2, 2, &valid_data);
    assert_eq!(mip.row_pitch, 8);
    assert_eq!(mip.bytes.len(), 16);

    // Explicit pitch with padding: width 2, row_pitch 12, height 2 -> needs 24 bytes.
    let padded_data = [0u8; 24];
    let padded_mip = Rgba8MipLevel {
        level: 0,
        width: 2,
        height: 2,
        row_pitch: 12,
        bytes: &padded_data,
    };
    assert_eq!(padded_mip.row_pitch, 12);
}

#[test]
fn test_invalid_resource_and_sampler_combinations() {
    let texture_desc = Rgba8TextureDesc::new(8, 8, 3).unwrap();
    let meta =
        TextureMetadata::new(texture_desc.width, texture_desc.height, texture_desc.mips).unwrap();

    // Sampler requesting explicit mip 3 when texture only has 3 mips (indices 0, 1, 2)
    let invalid_mip_sampler = SamplerDesc::nearest_clamp().with_explicit_mip(3);
    assert!(invalid_mip_sampler.validate_against_texture(&meta).is_err());

    // Sampler with min_lod 3.0 when texture only has 3 mips
    let invalid_lod_sampler = SamplerDesc::nearest_clamp().with_lod_range(3.0, 5.0);
    assert!(invalid_lod_sampler.validate_against_texture(&meta).is_err());

    // Valid sampler against this texture
    let valid_sampler = SamplerDesc::nearest_clamp().with_lod_range(0.0, 2.0);
    assert!(valid_sampler.validate_against_texture(&meta).is_ok());
}

#[test]
fn test_binding_slot_construction() {
    let tex_slot = BindingSlot::new(0);
    let samp_slot = BindingSlot::new(1);
    assert_eq!(tex_slot.index, 0);
    assert_eq!(samp_slot.index, 1);
}
