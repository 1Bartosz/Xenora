// SPDX-License-Identifier: GPL-3.0-or-later

use xenora_metal::NativeCompute;
use xenora_shader::{Architecture, PIXEL_ENTRY_POINT, VERTEX_ENTRY_POINT, compile_graphics};

use crate::interp::FaultKind;

pub(super) const INPUT_BYTES: usize = 2 * 3 * 4 * 4;
const SIZE: [usize; 2] = [4, 3];
pub(super) const OUTPUT_BYTES: usize = SIZE[0] * SIZE[1] * 4;

#[derive(Clone, Copy)]
pub(super) enum Program {
    Constant,
    Varying,
}

impl Program {
    pub(super) fn parse(id: u32) -> Result<Self, FaultKind> {
        match id {
            1 => Ok(Self::Constant),
            2 => Ok(Self::Varying),
            _ => Err(FaultKind::InvalidServiceArgument),
        }
    }
}

pub(super) fn run(program: Program, bytes: &[u8]) -> Result<Vec<u8>, FaultKind> {
    let (vertex, pixel): (&[u8], &[u8]) = match program {
        Program::Constant => (
            include_bytes!("../../../../fixtures/rdna2/x9-constant-vs.bin"),
            include_bytes!("../../../../fixtures/rdna2/x9-constant-ps.bin"),
        ),
        Program::Varying => (
            include_bytes!("../../../../fixtures/rdna2/x9-varying-vs.bin"),
            include_bytes!("../../../../fixtures/rdna2/x9-varying-ps.bin"),
        ),
    };
    let mut vectors = [[0.0; 4]; 6];
    for (component, word) in vectors.iter_mut().flatten().zip(bytes.as_chunks::<4>().0) {
        *component = f32::from_le_bytes(*word);
    }
    if !vectors.iter().flatten().all(|value| value.is_finite()) {
        return Err(FaultKind::InvalidServiceArgument);
    }
    let [vertices, colors] = vectors.as_chunks::<3>().0 else {
        return Err(FaultKind::InvalidResourceView);
    };
    let shader = compile_graphics(Architecture::Gfx1030, vertex, pixel).map_err(|error| {
        eprintln!("X9 graphics compile: {error:?}");
        FaultKind::DrawFailed
    })?;
    (|| {
        NativeCompute::new()?.render_rgba8(
            &shader.msl,
            [VERTEX_ENTRY_POINT, PIXEL_ENTRY_POINT],
            vertices,
            colors,
            SIZE,
            [0, 0, 0, 0],
        )
    })()
    .map_err(|error| {
        eprintln!("X9 draw: {error}");
        FaultKind::DrawFailed
    })
}
