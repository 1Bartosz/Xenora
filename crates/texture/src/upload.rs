// SPDX-License-Identifier: GPL-3.0-or-later

use super::{BLOCK_BYTES, TextureError, mip_blocks};

/// A layer-major BC1 upload with one common pitch for every mip.
/// Each mip occupies exactly one slice, including any trailing padding.
#[derive(Clone, Copy, Debug)]
pub struct Bc1UploadDesc {
    pub width: usize,
    pub height: usize,
    pub mips: u32,
    pub layers: usize,
    pub format: u32,
    pub usage: u32,
    pub row_pitch: usize,
    pub slice_pitch: usize,
}

#[derive(Debug)]
pub struct Bc1Upload<'a> {
    bytes: &'a [u8],
    desc: Bc1UploadDesc,
}

#[derive(Debug)]
pub struct Bc1Subresource<'a> {
    pub bytes: &'a [u8],
    pub width: usize,
    pub height: usize,
    pub row_pitch: usize,
}

impl<'a> Bc1Upload<'a> {
    pub fn new(bytes: &'a [u8], desc: Bc1UploadDesc) -> Result<Self, TextureError> {
        let max_mips = usize::BITS - desc.width.max(desc.height).leading_zeros();
        if desc.format != 1
            || desc.usage != 1
            || desc.width == 0
            || desc.height == 0
            || desc.layers == 0
            || desc.mips == 0
            || desc.mips > max_mips
        {
            return Err(TextureError::InvalidLayout);
        }
        let (blocks_x, blocks_y) = mip_blocks(desc.width, desc.height, 0);
        let tight_row = blocks_x
            .checked_mul(BLOCK_BYTES)
            .ok_or(TextureError::SizeOverflow)?;
        if desc.row_pitch < tight_row || !desc.row_pitch.is_multiple_of(BLOCK_BYTES) {
            return Err(TextureError::InvalidLayout);
        }
        let minimum_slice = desc
            .row_pitch
            .checked_mul(blocks_y)
            .ok_or(TextureError::SizeOverflow)?;
        if desc.slice_pitch < minimum_slice || !desc.slice_pitch.is_multiple_of(BLOCK_BYTES) {
            return Err(TextureError::InvalidLayout);
        }
        let size = desc
            .slice_pitch
            .checked_mul(desc.mips as usize)
            .and_then(|size| size.checked_mul(desc.layers))
            .ok_or(TextureError::SizeOverflow)?;
        if bytes.len() != size {
            return Err(TextureError::InvalidByteLength);
        }
        Ok(Self { bytes, desc })
    }

    pub fn subresource(&self, layer: usize, mip: u32) -> Result<Bc1Subresource<'a>, TextureError> {
        if layer >= self.desc.layers || mip >= self.desc.mips {
            return Err(TextureError::OutOfBounds);
        }
        // Construction proved that the complete array fits in the source span.
        let offset = (layer * self.desc.mips as usize + mip as usize) * self.desc.slice_pitch;
        let width = (self.desc.width >> mip).max(1);
        let height = (self.desc.height >> mip).max(1);
        let rows = height.div_ceil(4);
        Ok(Bc1Subresource {
            bytes: &self.bytes[offset..offset + rows * self.desc.row_pitch],
            width,
            height,
            row_pitch: self.desc.row_pitch,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DESC: Bc1UploadDesc = Bc1UploadDesc {
        width: 5,
        height: 5,
        mips: 2,
        layers: 2,
        format: 1,
        usage: 1,
        row_pitch: 24,
        slice_pitch: 56,
    };

    #[test]
    fn selects_mips_and_layers_with_partial_blocks_and_padding() {
        let mut bytes = [0; 224];
        bytes[144..152].fill(0x9a); // Layer 1, mip 0, row 1, block 1.
        bytes[168..176].copy_from_slice(&[0, 0, 255, 255, 255, 255, 255, 255]);
        let upload = Bc1Upload::new(&bytes, DESC).unwrap();
        let base = upload.subresource(1, 0).unwrap();
        assert_eq!((base.width, base.height, base.row_pitch), (5, 5, 24));
        assert_eq!(base.bytes.len(), 48);
        assert_eq!(base.bytes[32..40], [0x9a; 8]);
        let small = upload.subresource(1, 1).unwrap();
        assert_eq!((small.width, small.height, small.bytes.len()), (2, 2, 24));
        assert_eq!(
            crate::decode_bc1(small.bytes[..8].try_into().unwrap()),
            [[0; 4]; 16]
        );
        assert_eq!(upload.subresource(0, 1).unwrap().bytes, [0; 24]);
        assert_eq!(
            upload.subresource(0, 2).unwrap_err(),
            TextureError::OutOfBounds
        );
        assert_eq!(
            upload.subresource(2, 0).unwrap_err(),
            TextureError::OutOfBounds
        );
    }

    #[test]
    fn rejects_invalid_uploads_before_indexing() {
        let bytes = [0; 224];
        for desc in [
            Bc1UploadDesc { format: 2, ..DESC },
            Bc1UploadDesc { usage: 2, ..DESC },
            Bc1UploadDesc { mips: 4, ..DESC },
            Bc1UploadDesc { layers: 0, ..DESC },
            Bc1UploadDesc {
                row_pitch: 8,
                ..DESC
            },
            Bc1UploadDesc {
                row_pitch: 17,
                ..DESC
            },
            Bc1UploadDesc {
                slice_pitch: 40,
                ..DESC
            },
            Bc1UploadDesc {
                slice_pitch: 49,
                ..DESC
            },
        ] {
            assert_eq!(
                Bc1Upload::new(&bytes, desc).unwrap_err(),
                TextureError::InvalidLayout
            );
        }
        assert_eq!(
            Bc1Upload::new(&bytes[..223], DESC).unwrap_err(),
            TextureError::InvalidByteLength
        );
        for desc in [
            Bc1UploadDesc {
                width: usize::MAX,
                ..DESC
            },
            Bc1UploadDesc {
                row_pitch: usize::MAX - 7,
                ..DESC
            },
            Bc1UploadDesc {
                layers: usize::MAX,
                ..DESC
            },
        ] {
            assert_eq!(
                Bc1Upload::new(&bytes, desc).unwrap_err(),
                TextureError::SizeOverflow
            );
        }
    }
}
