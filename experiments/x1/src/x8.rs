// SPDX-License-Identifier: GPL-3.0-or-later

use xenora_metal::{GuestBuffer, GuestBufferView, NativeCompute};
use xenora_shader::{Architecture, ENTRY_POINT, compile};

use crate::{interp::FaultKind, mem::GuestMemory, x7::WORDS};

const BYTE_LEN: usize = WORDS * 4;
const XOR_SHADER: &[u8] =
    include_bytes!("../../../crates/metal/tests/fixtures/guest-buffer-xor.bin");

pub(crate) fn compute(
    memory: &mut GuestMemory,
    address: u64,
    descriptor: u64,
) -> Result<[u32; WORDS], FaultKind> {
    if !address.is_multiple_of(4) || !descriptor.is_multiple_of(4) {
        return Err(FaultKind::InvalidServiceArgument);
    }
    let fields = memory.read_bytes(descriptor, 16)?;
    let mut fields = fields
        .as_chunks::<4>()
        .0
        .iter()
        .copied()
        .map(u32::from_le_bytes);
    // Command 1 selects XOR, handle 0 names the single caller-supplied resource,
    // and binding 0 is the translated shader's input slot.
    if fields.next() != Some(1)
        || fields.next() != Some(0)
        || fields.next() != Some(0)
        || fields.next() != Some(WORDS as u32)
    {
        return Err(FaultKind::InvalidServiceArgument);
    }
    memory.check_writable(address, BYTE_LEN)?;
    let bytes = memory.read_bytes(address, BYTE_LEN)?;

    let readback = run_words(&bytes, 0, BYTE_LEN)?;
    let words: [u32; WORDS] = readback.try_into().map_err(|_| FaultKind::ComputeFailed)?;
    let mut output = [0; BYTE_LEN];
    for (slot, word) in output.as_chunks_mut::<4>().0.iter_mut().zip(words) {
        *slot = word.to_le_bytes();
    }
    memory.write_bytes(address, &output)?;
    Ok(words)
}

pub(crate) fn run_words(
    bytes: &[u8],
    byte_offset: usize,
    byte_len: usize,
) -> Result<Vec<u32>, FaultKind> {
    (|| -> Result<_, String> {
        let shader = compile(Architecture::Gfx1030, XOR_SHADER).map_err(|e| format!("{e:?}"))?;
        let context = NativeCompute::new().map_err(|e| e.to_string())?;
        context
            .run_guest_words(
                &shader.msl,
                ENTRY_POINT,
                &[GuestBuffer {
                    format: GuestBuffer::U32_LE,
                    bytes,
                }],
                GuestBufferView {
                    handle: 0,
                    binding: 0,
                    byte_offset,
                    byte_len,
                },
            )
            .map_err(|e| e.to_string())
    })()
    .map_err(|error| {
        eprintln!("X8 compute: {error}");
        FaultKind::ComputeFailed
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mem::{MemFault, Perm};

    #[test]
    fn rejects_invalid_descriptor_and_output_ranges() {
        let mut memory = GuestMemory::new();
        assert!(memory.map(0x1000, 16, Perm::RW, 0));
        memory.store_u32(0x1000, 1).unwrap();
        memory.store_u32(0x100c, WORDS as u32).unwrap();
        assert!(memory.map(0x2000, 8, Perm::RW, 0xA5));
        assert!(memory.map(0x2008, 4, Perm::RW, 0x5A));
        assert!(memory.map(0x3000, BYTE_LEN, Perm::READ, 0xFF));
        for (address, descriptor, expected) in [
            (0x2000, 0x1004, MemFault::CrossBoundary),
            (0x2000, 0x1000, MemFault::CrossBoundary),
            (0x3000, 0x1000, MemFault::NotWritable),
        ] {
            assert_eq!(
                compute(&mut memory, address, descriptor),
                Err(FaultKind::MemFault(expected))
            );
        }
        assert_eq!(memory.read_bytes(0x2000, 8).unwrap(), [0xA5; 8]);
        assert_eq!(memory.read_bytes(0x2008, 4).unwrap(), [0x5A; 4]);
        assert_eq!(
            memory.read_bytes(0x3000, BYTE_LEN).unwrap(),
            [0xFF; BYTE_LEN]
        );
    }
}
