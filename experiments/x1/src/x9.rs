// SPDX-License-Identifier: GPL-3.0-or-later

use std::ops::Range;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use crate::{interp::FaultKind, mem::GuestMemory};

mod draw;
mod texture;

const SLOT_COUNT: usize = 8;
const SLOT_BITS: u32 = 3;
const MAX_BUFFER_BYTES: u32 = 4096;
const REQUEST_BYTES: usize = 20;
const MAX_READBACKS: usize = 16;
static NEXT_GENERATION: AtomicU32 = AtomicU32::new(1);

#[derive(Clone, Copy)]
enum ResourceKind {
    Words,
    Vertices,
    Bc1(texture::Upload),
}

impl ResourceKind {
    fn parse(kind: u32) -> Result<Self, FaultKind> {
        match kind {
            1 => Ok(Self::Words),
            2 => Ok(Self::Vertices),
            _ => Err(FaultKind::InvalidServiceArgument),
        }
    }
}

struct Resource {
    kind: ResourceKind,
    bytes: Arc<[u8]>,
}

#[derive(Default)]
struct Slot {
    generation: u32,
    resource: Option<Resource>,
}

#[derive(Clone, Copy)]
enum Operation {
    Xor,
    Draw(draw::Program),
    Texture(texture::Read),
}

struct Pending {
    bytes: Arc<[u8]>,
    range: Range<usize>,
    operation: Operation,
}

impl Pending {
    fn output_len(&self) -> usize {
        match self.operation {
            Operation::Xor => self.range.len(),
            Operation::Draw(_) => draw::OUTPUT_BYTES,
            Operation::Texture(read) => read.output_len,
        }
    }

    fn run(&self) -> Result<Vec<u8>, FaultKind> {
        match self.operation {
            Operation::Xor => {
                Ok(
                    crate::x8::run_words(&self.bytes, self.range.start, self.range.len())?
                        .into_iter()
                        .flat_map(u32::to_le_bytes)
                        .collect(),
                )
            }
            Operation::Draw(program) => draw::run(program, &self.bytes[self.range.clone()]),
            Operation::Texture(read) => read.run(&self.bytes),
        }
    }
}

#[derive(Default)]
pub(crate) struct Resources {
    slots: [Slot; SLOT_COUNT],
    pending: Option<Pending>,
    pub(crate) readbacks: Vec<Vec<u8>>,
}

fn aligned(address: u64) -> Result<(), FaultKind> {
    if address.is_multiple_of(4) {
        Ok(())
    } else {
        Err(FaultKind::InvalidServiceArgument)
    }
}

fn view(offset: u32, length: u32, capacity: usize) -> Result<Range<usize>, FaultKind> {
    let end = offset
        .checked_add(length)
        .ok_or(FaultKind::InvalidResourceView)? as usize;
    if length == 0 || !offset.is_multiple_of(4) || !length.is_multiple_of(4) || end > capacity {
        return Err(FaultKind::InvalidResourceView);
    }
    Ok(offset as usize..end)
}

impl Resources {
    fn create(
        &mut self,
        memory: &GuestMemory,
        addr: u64,
        length: u32,
        kind: u32,
    ) -> Result<u32, FaultKind> {
        let kind = ResourceKind::parse(kind)?;
        aligned(addr)?;
        let range = view(0, length, MAX_BUFFER_BYTES as usize)?;
        let bytes = memory.read_bytes(addr, range.len())?;
        self.install(Resource {
            kind,
            bytes: bytes.into(),
        })
    }

    fn install(&mut self, resource: Resource) -> Result<u32, FaultKind> {
        let index = self
            .slots
            .iter()
            .position(|slot| slot.resource.is_none())
            .ok_or(FaultKind::ResourceLimit)?;
        // Never recycle a generation, including across guest runs in the same host process.
        let generation = NEXT_GENERATION
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |next| {
                (next <= u32::MAX >> SLOT_BITS).then_some(next + 1)
            })
            .map_err(|_| FaultKind::ResourceLimit)?;
        self.slots[index] = Slot {
            generation,
            resource: Some(resource),
        };
        Ok((generation << SLOT_BITS) | index as u32)
    }

    fn index(&self, handle: u32) -> Result<usize, FaultKind> {
        let index = (handle & ((1 << SLOT_BITS) - 1)) as usize;
        let generation = handle >> SLOT_BITS;
        let slot = &self.slots[index];
        if generation == 0 || generation > slot.generation {
            return Err(FaultKind::InvalidResource);
        }
        if generation != slot.generation || slot.resource.is_none() {
            return Err(FaultKind::StaleResource);
        }
        Ok(index)
    }

    fn release(&mut self, handle: u32) -> Result<u32, FaultKind> {
        let index = self.index(handle)?;
        self.slots[index].resource = None;
        Ok(0)
    }

    fn update(
        &mut self,
        memory: &GuestMemory,
        addr: u64,
        handle: u32,
        offset: u32,
        length: u32,
    ) -> Result<u32, FaultKind> {
        let index = self.index(handle)?;
        let buffer = self.slots[index]
            .resource
            .as_mut()
            .ok_or(FaultKind::StaleResource)?;
        if matches!(buffer.kind, ResourceKind::Bc1(_)) {
            return Err(FaultKind::InvalidResourceView);
        }
        aligned(addr)?;
        let range = view(offset, length, buffer.bytes.len())?;
        let bytes = memory.read_bytes(addr, range.len())?;
        Arc::make_mut(&mut buffer.bytes)[range].copy_from_slice(&bytes);
        Ok(length)
    }

    fn queue(
        &mut self,
        operation: Operation,
        handle: u32,
        offset: u32,
        length: u32,
    ) -> Result<u32, FaultKind> {
        if self.pending.is_some() {
            return Err(FaultKind::SubmissionPending);
        }
        let index = self.index(handle)?;
        let buffer = self.slots[index]
            .resource
            .as_ref()
            .ok_or(FaultKind::StaleResource)?;
        match (operation, buffer.kind) {
            (Operation::Xor, ResourceKind::Words) => {}
            (Operation::Draw(_), ResourceKind::Vertices) if length == draw::INPUT_BYTES as u32 => {}
            _ => return Err(FaultKind::InvalidResourceView),
        }
        let range = view(offset, length, buffer.bytes.len())?;
        self.pending = Some(Pending {
            bytes: Arc::clone(&buffer.bytes),
            range,
            operation,
        });
        Ok(length)
    }

    fn create_texture(
        &mut self,
        memory: &GuestMemory,
        addr: u64,
        length: u32,
    ) -> Result<u32, FaultKind> {
        aligned(addr)?;
        let (upload, bytes) = texture::Upload::from_guest(memory, addr, length)?;
        self.install(Resource {
            kind: ResourceKind::Bc1(upload),
            bytes: bytes.into(),
        })
    }

    fn queue_texture(&mut self, handle: u32, layer: u32, mip: u32) -> Result<u32, FaultKind> {
        if self.pending.is_some() {
            return Err(FaultKind::SubmissionPending);
        }
        let index = self.index(handle)?;
        let resource = self.slots[index]
            .resource
            .as_ref()
            .ok_or(FaultKind::StaleResource)?;
        let ResourceKind::Bc1(upload) = resource.kind else {
            return Err(FaultKind::InvalidResourceView);
        };
        let read = upload.select(&resource.bytes, layer as usize, mip)?;
        self.pending = Some(Pending {
            bytes: Arc::clone(&resource.bytes),
            range: 0..resource.bytes.len(),
            operation: Operation::Texture(read),
        });
        Ok(read.output_len as u32)
    }

    fn finish(
        &mut self,
        memory: &mut GuestMemory,
        addr: u64,
        length: u32,
    ) -> Result<u32, FaultKind> {
        let pending = self.pending.as_ref().ok_or(FaultKind::NoSubmission)?;
        aligned(addr)?;
        if length as usize != pending.output_len() {
            return Err(FaultKind::InvalidServiceArgument);
        }
        memory.check_writable(addr, length as usize)?;
        if self.readbacks.len() == MAX_READBACKS {
            return Err(FaultKind::ResourceLimit);
        }
        let readback = pending.run()?;
        if readback.len() != length as usize {
            return Err(FaultKind::InvalidServiceArgument);
        }
        memory.write_bytes(addr, &readback)?;
        self.pending = None;
        self.readbacks.push(readback);
        Ok(length)
    }

    pub(crate) fn invoke(
        &mut self,
        memory: &mut GuestMemory,
        request: u64,
        data: u64,
    ) -> Result<u64, FaultKind> {
        aligned(request)?;
        let bytes = memory.read_bytes(request, REQUEST_BYTES)?;
        let words = bytes.as_chunks::<4>().0;
        let [opcode, handle, offset, length, kind] = <&[[u8; 4]; 5]>::try_from(words)
            .map_err(|_| FaultKind::InvalidServiceArgument)?
            .map(u32::from_le_bytes);
        let result = match (opcode, handle, offset, length, kind, data) {
            (1, 0, 0, length, kind, addr) => self.create(memory, addr, length, kind),
            (2, handle, offset, length, 0, addr) => {
                self.update(memory, addr, handle, offset, length)
            }
            (3, handle, 0, 0, 0, 0) => self.release(handle),
            (4, handle, offset, length, 0, 0) => self.queue(Operation::Xor, handle, offset, length),
            (5, 0, 0, length, 0, addr) => self.finish(memory, addr, length),
            (6, handle, offset, length, program, 0) => self.queue(
                Operation::Draw(draw::Program::parse(program)?),
                handle,
                offset,
                length,
            ),
            (7, 0, 0, length, 0, addr) => self.create_texture(memory, addr, length),
            (8, handle, layer, mip, 0, 0) => self.queue_texture(handle, layer, mip),
            _ => Err(FaultKind::InvalidServiceArgument),
        }?;
        Ok(u64::from(result))
    }
}

#[cfg(test)]
mod tests;
