// SPDX-License-Identifier: GPL-3.0-or-later

use xenora_metal::NativeCompute;
use xenora_texture::{Bc1Upload, Bc1UploadDesc};

use crate::{interp::FaultKind, mem::GuestMemory};

const DESCRIPTOR_BYTES: usize = 32;
const MAX_READ_PIXELS: usize = 4096;

#[derive(Clone, Copy)]
pub(super) struct Upload(Bc1UploadDesc);

impl Upload {
    pub(super) fn from_guest(
        memory: &GuestMemory,
        addr: u64,
        length: u32,
    ) -> Result<(Self, Vec<u8>), FaultKind> {
        if !(1..=super::MAX_BUFFER_BYTES).contains(&length) {
            return Err(FaultKind::InvalidResourceView);
        }
        let mut bytes = memory.read_bytes(addr, DESCRIPTOR_BYTES + length as usize)?;
        let fields = bytes[..DESCRIPTOR_BYTES].as_chunks::<4>().0;
        let [
            width,
            height,
            mips,
            layers,
            format,
            usage,
            row_pitch,
            slice_pitch,
        ] = <&[[u8; 4]; 8]>::try_from(fields)
            .map_err(|_| FaultKind::InvalidServiceArgument)?
            .map(u32::from_le_bytes);
        let upload = Self(Bc1UploadDesc {
            width: width as usize,
            height: height as usize,
            mips,
            layers: layers as usize,
            format,
            usage,
            row_pitch: row_pitch as usize,
            slice_pitch: slice_pitch as usize,
        });
        let texels = bytes.split_off(DESCRIPTOR_BYTES);
        upload.validate(&texels)?;
        Ok((upload, texels))
    }

    fn validate(self, bytes: &[u8]) -> Result<Bc1Upload<'_>, FaultKind> {
        Bc1Upload::new(bytes, self.0).map_err(|_| FaultKind::InvalidResourceView)
    }

    pub(super) fn select(self, bytes: &[u8], layer: usize, mip: u32) -> Result<Read, FaultKind> {
        let subresource = self
            .validate(bytes)?
            .subresource(layer, mip)
            .map_err(|_| FaultKind::InvalidResourceView)?;
        let pixels = subresource
            .width
            .checked_mul(subresource.height)
            .filter(|pixels| (1..=MAX_READ_PIXELS).contains(pixels))
            .ok_or(FaultKind::InvalidResourceView)?;
        Ok(Read {
            upload: self,
            layer,
            mip,
            output_len: pixels * 4,
        })
    }
}

#[derive(Clone, Copy)]
pub(super) struct Read {
    upload: Upload,
    layer: usize,
    mip: u32,
    pub(super) output_len: usize,
}

impl Read {
    pub(super) fn run(self, bytes: &[u8]) -> Result<Vec<u8>, FaultKind> {
        let upload = self.upload.validate(bytes)?;
        (|| NativeCompute::new()?.read_bc1_upload(&upload, self.layer, self.mip))()
            .map(|words| words.into_iter().flat_map(u32::to_le_bytes).collect())
            .map_err(|error| {
                eprintln!("X9 texture read: {error}");
                FaultKind::TextureFailed
            })
    }
}
