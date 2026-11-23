// SPDX-License-Identifier: GPL-3.0-or-later

use std::fs::File;
use std::io::Read;
use std::path::Path;
use std::process::ExitCode;

use crate::interp::{self, FaultKind, Services, State};
use crate::loader::{self, ImportDirectories, LoadedImage};
use crate::mem::GuestMemory;

const MAX_FILE_BYTES: u64 = 64 * 1024 * 1024;
const STACK_BASE: u64 = 0x7FFF_F000;
const STACK_SIZE: usize = 0x2000;
const DESCRIPTOR_SIZE: usize = 20;
const MODULE: &[u8] = b"XENORA-TEST.dll";
pub(super) const WORDS: usize = 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Service {
    Transform,
    #[cfg(feature = "metal")]
    Compute,
    Report,
    #[cfg(feature = "metal")]
    Resource,
}

#[derive(Clone, Copy)]
pub(crate) enum Profile {
    Imports,
    #[cfg(feature = "metal")]
    Resources,
}

impl Profile {
    fn label(self) -> &'static str {
        match self {
            Self::Imports => "X7",
            #[cfg(feature = "metal")]
            Self::Resources => "X9",
        }
    }

    fn budget(self) -> usize {
        match self {
            Self::Imports => 128,
            #[cfg(feature = "metal")]
            Self::Resources => 512,
        }
    }
}

#[derive(Debug)]
struct Binding {
    slot: u64,
    service: Service,
}

#[derive(Debug, PartialEq, Eq)]
struct Execution {
    guest_exit: u32,
    calls: Vec<(Service, [u32; WORDS])>,
    #[cfg(feature = "metal")]
    readbacks: Vec<Vec<u8>>,
}

fn readonly(image: &LoadedImage, rva: u32, len: usize) -> Result<&[u8], String> {
    image
        .mem
        .read_readonly(image.image_base + u64::from(rva), len)
        .map_err(|fault| format!("invalid import data at RVA {rva:#x}: {fault:?}"))
}

fn import_name(image: &LoadedImage, mut rva: u32) -> Result<Vec<u8>, String> {
    let mut name = Vec::new();
    let start = rva;
    for _ in 0..128 {
        let byte = readonly(image, rva, 1)?[0];
        if byte == 0 {
            // Require the whole name to occupy a single read-only region.
            readonly(image, start, name.len() + 1)?;
            return Ok(name);
        }
        if !byte.is_ascii_graphic() {
            break;
        }
        name.push(byte);
        rva = rva.checked_add(1).ok_or("import name RVA overflow")?;
    }
    Err("invalid or unterminated import name".into())
}

fn bindings(
    image: &LoadedImage,
    directories: ImportDirectories,
    profile: Profile,
) -> Result<Vec<Binding>, String> {
    let ImportDirectories {
        descriptors,
        address_table,
    } = directories;
    // X7 has one named module and at most two named imports, each unique.
    if descriptors.size != (2 * DESCRIPTOR_SIZE) as u32
        || !address_table.rva.is_multiple_of(8)
        || !matches!(address_table.size, 16 | 24)
    {
        return Err("unsupported import descriptor or address-table size".into());
    }
    let table = readonly(image, descriptors.rva, descriptors.size as usize)?;
    if table[DESCRIPTOR_SIZE..].iter().any(|&b| b != 0) {
        return Err("expected one import descriptor followed by a terminator".into());
    }
    let mut fields = table[..DESCRIPTOR_SIZE]
        .as_chunks::<4>()
        .0
        .iter()
        .copied()
        .map(u32::from_le_bytes);
    let lookup_rva = fields.next().ok_or("missing import lookup table")?;
    let timestamp = fields.next().ok_or("missing import timestamp")?;
    let forwarder = fields.next().ok_or("missing forwarder chain")?;
    let name_rva = fields.next().ok_or("missing module name")?;
    let iat_rva = fields.next().ok_or("missing import address table")?;
    if timestamp != 0
        || forwarder != 0
        || iat_rva != address_table.rva
        || lookup_rva == 0
        || !lookup_rva.is_multiple_of(8)
    {
        return Err("unsupported import descriptor".into());
    }
    let module = import_name(image, name_rva)?;
    if !module.eq_ignore_ascii_case(MODULE) {
        return Err(format!(
            "unresolved import module {}",
            String::from_utf8_lossy(&module)
        ));
    }
    let lookup = readonly(image, lookup_rva, address_table.size as usize)?;
    let iat = readonly(image, iat_rva, address_table.size as usize)?;
    if lookup != iat || lookup[lookup.len() - 8..] != [0; 8] {
        return Err("mismatched or unterminated import tables".into());
    }

    let mut result = Vec::new();
    for (index, thunk) in lookup[..lookup.len() - 8]
        .as_chunks::<8>()
        .0
        .iter()
        .enumerate()
    {
        if thunk[4..] != [0; 4] {
            return Err("only named import thunks are supported".into());
        }
        let hint_rva = u32::from_le_bytes([thunk[0], thunk[1], thunk[2], thunk[3]]);
        readonly(image, hint_rva, 2)?;
        let name = import_name(
            image,
            hint_rva.checked_add(2).ok_or("import hint RVA overflow")?,
        )?;
        let service = match name.as_slice() {
            b"xenora_transform" if matches!(profile, Profile::Imports) => Service::Transform,
            #[cfg(feature = "metal")]
            b"xenora_compute" if matches!(profile, Profile::Imports) => Service::Compute,
            #[cfg(feature = "metal")]
            b"xenora_resource" if matches!(profile, Profile::Resources) => Service::Resource,
            b"xenora_report" => Service::Report,
            _ => {
                return Err(format!(
                    "unresolved import XENORA-TEST.dll!{}",
                    String::from_utf8_lossy(&name)
                ));
            }
        };
        if result
            .iter()
            .any(|binding: &Binding| binding.service == service)
        {
            return Err("duplicate synthetic service import".into());
        }
        result.push(Binding {
            slot: image.image_base + u64::from(iat_rva) + (index * 8) as u64,
            service,
        });
    }
    Ok(result)
}

fn invoke(
    service: Service,
    state: &State,
    memory: &mut GuestMemory,
    calls: &mut Vec<(Service, [u32; WORDS])>,
) -> Result<u64, FaultKind> {
    let address = state.gpr[1]; // RCX
    let count = state.gpr[2] as u32; // EDX
    if count != WORDS as u32 || !address.is_multiple_of(4) {
        return Err(FaultKind::InvalidServiceArgument);
    }
    let bytes = memory.read_bytes(address, WORDS * 4)?;
    let mut words = [0_u32; WORDS];
    for (word, bytes) in words.iter_mut().zip(bytes.as_chunks::<4>().0) {
        *word = u32::from_le_bytes(*bytes);
    }
    let value = match service {
        Service::Transform => {
            words = words.map(|word| word.wrapping_mul(2));
            let mut output = [0; WORDS * 4];
            for (slot, word) in output.as_chunks_mut::<4>().0.iter_mut().zip(words) {
                slot.copy_from_slice(&word.to_le_bytes());
            }
            // One checked operation for the entire output, including boundary
            // and write-permission validation before any guest bytes change.
            memory.write_bytes(address, &output)?;
            WORDS as u32
        }
        #[cfg(feature = "metal")]
        Service::Compute | Service::Resource => return Err(FaultKind::UnknownService),
        Service::Report => words.into_iter().fold(0u32, u32::wrapping_add),
    };
    calls.push((service, words));
    Ok(u64::from(value))
}

fn execute(bytes: &[u8], profile: Profile) -> Result<Execution, String> {
    let (mut image, directories) =
        loader::load_imports(bytes).map_err(|error| format!("PE load: {error:?}"))?;
    let bindings = bindings(&image, directories, profile)?;
    let mut state = State::new(image.entry);
    state.gpr[4] = image
        .mem
        .alloc_stack(STACK_BASE, STACK_SIZE)
        .ok_or("stack allocation failed")?;
    let mut calls = Vec::new();
    #[cfg(feature = "metal")]
    let mut compute_consumed = false;
    #[cfg(feature = "metal")]
    let mut resources = crate::x9::Resources::default();
    // Resolve the validated guest slot against the host-side bindings.
    let mut call = |slot, state: &State, memory: &mut GuestMemory| {
        let binding = bindings
            .iter()
            .find(|binding| binding.slot == slot)
            .ok_or(FaultKind::UnboundImport)?;
        #[cfg(feature = "metal")]
        if binding.service == Service::Compute {
            if compute_consumed {
                return Err(FaultKind::StaleResource);
            }
            let words = crate::x8::compute(memory, state.gpr[1], state.gpr[2])?;
            compute_consumed = true;
            calls.push((binding.service, words));
            return Ok(WORDS as u64);
        }
        #[cfg(feature = "metal")]
        if binding.service == Service::Resource {
            return resources.invoke(memory, state.gpr[1], state.gpr[2]);
        }
        invoke(binding.service, state, memory, &mut calls)
    };
    interp::run_with_services(
        &mut image.mem,
        &mut state,
        profile.budget(),
        Services {
            trap: None,
            import: Some(&mut call),
        },
        |_| {},
    )
    .map_err(|fault| format!("guest fault {:?} at RIP {:#x}", fault.kind, fault.rip))?;
    Ok(Execution {
        guest_exit: state.gpr[0] as u32,
        calls,
        #[cfg(feature = "metal")]
        readbacks: resources.readbacks,
    })
}

pub(crate) fn run(path: &Path, profile: Profile) -> ExitCode {
    let label = profile.label();
    let result = (|| -> Result<Execution, String> {
        let metadata =
            std::fs::metadata(path).map_err(|error| format!("{}: {error}", path.display()))?;
        if !metadata.is_file() {
            return Err("expected a regular guest file".into());
        }
        if metadata.len() > MAX_FILE_BYTES {
            return Err("guest file exceeds the size limit".into());
        }
        let file = File::open(path).map_err(|error| format!("{}: {error}", path.display()))?;
        let mut image = Vec::new();
        file.take(MAX_FILE_BYTES + 1)
            .read_to_end(&mut image)
            .map_err(|error| format!("{}: {error}", path.display()))?;
        if image.len() as u64 > MAX_FILE_BYTES {
            return Err("guest file exceeds the size limit".into());
        }
        execute(&image, profile)
    })();
    match result {
        Ok(execution) => {
            for (service, words) in execution.calls {
                let name = match service {
                    Service::Transform => "xenora_transform",
                    #[cfg(feature = "metal")]
                    Service::Compute => "xenora_compute",
                    Service::Report => "xenora_report",
                    #[cfg(feature = "metal")]
                    Service::Resource => "xenora_resource",
                };
                println!("{label} {name}: {words:?}");
            }
            #[cfg(feature = "metal")]
            for bytes in execution.readbacks {
                let hex: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
                println!("{label} readback: {hex}");
            }
            println!("{label} guest exit status: {}", execution.guest_exit);
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("{label}: {error}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mem::{MemFault, Perm};

    #[test]
    fn transform_rejects_read_only_and_cross_region_buffers_atomically() {
        let mut memory = GuestMemory::new();
        assert!(memory.map(0x1000, 8, Perm::RW, 0xA5));
        assert!(memory.map(0x1008, 4, Perm::RW, 0x5A));
        assert!(memory.map(0x2000, 12, Perm::READ, 0xFF));
        let mut state = State::new(0);
        state.gpr[2] = 3;
        let mut calls = Vec::new();
        for (address, fault) in [
            (0x1000, MemFault::CrossBoundary),
            (0x2000, MemFault::NotWritable),
        ] {
            state.gpr[1] = address;
            assert_eq!(
                invoke(Service::Transform, &state, &mut memory, &mut calls),
                Err(FaultKind::MemFault(fault))
            );
        }
        assert_eq!(memory.read_bytes(0x1000, 8).unwrap(), [0xA5; 8]);
        assert_eq!(memory.read_bytes(0x1008, 4).unwrap(), [0x5A; 4]);
        assert_eq!(memory.read_bytes(0x2000, 12).unwrap(), [0xFF; 12]);
        assert!(calls.is_empty());
    }

    #[test]
    fn invalid_word_count_and_alignment_fail_before_guest_access() {
        let mut memory = GuestMemory::new();
        let mut state = State::new(0);
        let mut calls = Vec::new();
        for (address, count) in [(0x1000, 4), (0x1001, 3)] {
            state.gpr[1] = address;
            state.gpr[2] = count;
            assert_eq!(
                invoke(Service::Report, &state, &mut memory, &mut calls),
                Err(FaultKind::InvalidServiceArgument)
            );
        }
        assert!(calls.is_empty());
    }
}
