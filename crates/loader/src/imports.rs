// SPDX-License-Identifier: GPL-3.0-or-later

//! Guest Import Table Binder and Host Service Dispatch Boundary for Xenora X15.
//!
//! Maps guest PE IAT (Import Address Table) entries to synthetic dispatch thunks,
//! validating the Windows x64 ABI calling convention and dispatching calls to typed
//! guest services without exposing raw host function pointers.

use crate::cpu::{CpuState, ExitReason, Register};
use crate::import_table::{ImportName, ImportTableError, MAX_NAME_LEN, parse_imports};
use crate::memory::{GuestMemory, MemoryError};
use crate::pe::ParsedPe;
use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, RwLock};

/// Base virtual address range reserved for synthetic guest import dispatch trampolines.
pub const IMPORT_TRAMPOLINE_BASE_VA: u64 = 0x0000_0007_F000_0000;
pub const IMPORT_TRAMPOLINE_STRIDE: u64 = 16;

/// Typed error resulting from import resolution or service dispatch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ImportError {
    Table(ImportTableError),
    UnresolvedImport { module: String, symbol: String },
    Memory(MemoryError),
}

impl fmt::Display for ImportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Table(e) => write!(f, "malformed PE imports: {e}"),
            Self::UnresolvedImport { module, symbol } => {
                write!(f, "unresolved import dependency: {module}!{symbol}")
            }
            Self::Memory(e) => write!(f, "guest memory error binding imports: {e}"),
        }
    }
}

impl std::error::Error for ImportError {}

/// Why a service could not be added to the registry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RegistryError {
    /// The key already names a different service.
    Duplicate {
        module: String,
        symbol: String,
    },
    /// An alias names a service that does not exist.
    UnknownTarget {
        module: String,
        symbol: String,
    },
    InvalidName {
        module: String,
        symbol: String,
    },
}

impl fmt::Display for RegistryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (problem, module, symbol) = match self {
            Self::Duplicate { module, symbol } => ("is already registered", module, symbol),
            Self::UnknownTarget { module, symbol } => ("is not registered", module, symbol),
            Self::InvalidName { module, symbol } => ("is not a valid name", module, symbol),
        };
        write!(f, "import {module}!{symbol} {problem}")
    }
}

impl std::error::Error for RegistryError {}

impl From<MemoryError> for ImportError {
    fn from(e: MemoryError) -> Self {
        Self::Memory(e)
    }
}

/// Windows x64 Calling Convention Argument Extractor.
///
/// In Microsoft x64 ABI:
/// - Arg 0: RCX
/// - Arg 1: RDX
/// - Arg 2: R8
/// - Arg 3: R9
/// - Arg 4+: Stack [RSP + 40 + (n-4)*8] (accounting for return address [RSP] and 32-byte shadow space [RSP+8..RSP+40]).
#[derive(Clone, Copy)]
pub struct Win64Args {
    pub rcx: u64,
    pub rdx: u64,
    pub r8: u64,
    pub r9: u64,
    pub rsp: u64,
}

impl Win64Args {
    pub fn new(cpu: &CpuState) -> Self {
        Self {
            rcx: cpu.get(Register::Rcx),
            rdx: cpu.get(Register::Rdx),
            r8: cpu.get(Register::R8),
            r9: cpu.get(Register::R9),
            rsp: cpu.get(Register::Rsp),
        }
    }

    pub fn get(&self, index: usize, memory: &GuestMemory) -> Result<u64, MemoryError> {
        match index {
            0 => Ok(self.rcx),
            1 => Ok(self.rdx),
            2 => Ok(self.r8),
            3 => Ok(self.r9),
            n => {
                let stack_offset = ((n - 4) as u64)
                    .checked_mul(8)
                    .and_then(|offset| offset.checked_add(40))
                    .ok_or(MemoryError::Overflow)?;
                let addr = self
                    .rsp
                    .checked_add(stack_offset)
                    .ok_or(MemoryError::Overflow)?;
                memory.read_u64(addr)
            }
        }
    }
}

/// Handler for a typed guest service.
/// Returns Ok(ret_val) to be placed into RAX, or Err(reason) to halt/fault execution.
pub type ServiceHandler = Box<
    dyn Fn(&Win64Args, &mut CpuState, &mut GuestMemory) -> Result<u64, ExitReason> + Send + Sync,
>;

/// Scope of a registered handler's implementation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ServiceStatus {
    Implemented,
    Partial,
    Stub,
}

impl ServiceStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Implemented => "implemented",
            Self::Partial => "partial",
            Self::Stub => "stub",
        }
    }
}

/// Registration entry for an imported symbol.
pub struct ServiceDescriptor {
    pub module: String,
    pub symbol: String,
    pub handler: ServiceHandler,
    pub status: ServiceStatus,
}

#[derive(Clone, Default)]
pub(crate) struct DynamicTrampolines(Arc<RwLock<HashMap<u64, usize>>>);

impl DynamicTrampolines {
    pub(crate) fn revoke(&self, indices: impl Iterator<Item = usize>) {
        let mut entries = self.0.write().expect("dynamic service trampolines");
        for index in indices {
            entries.remove(&trampoline_address(index));
        }
    }

    pub(crate) fn expose(&self, index: usize) -> u64 {
        let address = trampoline_address(index);
        self.0
            .write()
            .expect("dynamic service trampolines")
            .insert(address, index);
        address
    }

    fn get(&self, address: u64) -> Option<usize> {
        if address < IMPORT_TRAMPOLINE_BASE_VA {
            return None;
        }
        self.0
            .read()
            .expect("dynamic service trampolines")
            .get(&address)
            .copied()
    }
}

fn trampoline_address(index: usize) -> u64 {
    IMPORT_TRAMPOLINE_BASE_VA + index as u64 * IMPORT_TRAMPOLINE_STRIDE
}

const CALLBACK_RETURN: u64 = IMPORT_TRAMPOLINE_BASE_VA - IMPORT_TRAMPOLINE_STRIDE;
const MAX_CALLBACK_DEPTH: usize = 32;

pub(crate) struct GuestCallback {
    pub(crate) entry: u64,
    pub(crate) context: u64,
    pub(crate) cancelled: bool,
}

type CallbackDispatch = Box<
    dyn Fn(&Win64Args, &GuestMemory) -> Result<Option<GuestCallback>, ExitReason> + Send + Sync,
>;

#[derive(Clone)]
pub(crate) struct CallbackFrame {
    resume: u64,
    preserved: Nonvolatile,
}

/// Registry for resolving and binding import symbols to guest trampoline VAs.
pub struct ImportRegistry {
    // (lower-case module, name) -> service index. Symbol names keep their case.
    services: HashMap<(String, ImportName), usize>,
    handlers: Vec<ServiceDescriptor>,
    callback_dispatchers: HashMap<usize, CallbackDispatch>,
    dynamic_trampolines: DynamicTrampolines,
    trampoline_to_service: HashMap<u64, usize>, // Trampoline VA -> service_index
}

impl Default for ImportRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl ImportRegistry {
    pub fn new() -> Self {
        Self {
            services: HashMap::new(),
            handlers: Vec::new(),
            callback_dispatchers: HashMap::new(),
            trampoline_to_service: HashMap::new(),
            dynamic_trampolines: DynamicTrampolines::default(),
        }
    }

    /// The one lookup used by binding, inspection and reports. Module names
    /// compare without case, as Windows does; symbol names are exact. API sets
    /// resolve only through registered aliases.
    pub fn resolve(&self, module: &str, name: &ImportName) -> Option<&ServiceDescriptor> {
        self.index(module, name).map(|index| &self.handlers[index])
    }

    fn index(&self, module: &str, name: &ImportName) -> Option<usize> {
        self.services
            .get(&(module.to_ascii_lowercase(), name.clone()))
            .copied()
    }

    pub(crate) fn dynamic_trampolines(&self) -> DynamicTrampolines {
        self.dynamic_trampolines.clone()
    }

    pub(crate) fn service_index(&self, module: &str, symbol: &str) -> Option<usize> {
        self.index(module, &ImportName::Name(symbol.to_owned()))
    }

    pub(crate) fn canonical_exports(
        &self,
        module: &str,
    ) -> impl Iterator<Item = (&ImportName, usize)> {
        self.services
            .iter()
            .filter_map(move |((key, name), &index)| {
                let descriptor = &self.handlers[index];
                (key == module
                    && descriptor.module == *key
                    && descriptor.status != ServiceStatus::Stub)
                    .then_some((name, index))
            })
    }

    fn check_key(module: &str, name: &ImportName) -> Result<(), RegistryError> {
        let printable = |text: &str| {
            !text.is_empty()
                && text.len() <= MAX_NAME_LEN
                && text.bytes().all(|b| b.is_ascii_graphic())
        };
        let valid = printable(module)
            && match name {
                ImportName::Name(symbol) => printable(symbol),
                ImportName::Ordinal(_) => true,
            };
        if valid {
            Ok(())
        } else {
            Err(RegistryError::InvalidName {
                module: module.to_owned(),
                symbol: name.to_string(),
            })
        }
    }

    /// Check whether the exact import key has a binding, including stub handlers.
    pub fn has_binding(&self, module: &str, symbol: &str) -> bool {
        self.service_index(module, symbol).is_some()
    }

    /// Register a host-provided guest service under a name or an ordinal.
    pub fn try_register(
        &mut self,
        module: &str,
        name: ImportName,
        status: ServiceStatus,
        handler: ServiceHandler,
    ) -> Result<(), RegistryError> {
        Self::check_key(module, &name)?;
        let key = (module.to_ascii_lowercase(), name);
        if self.services.contains_key(&key) {
            return Err(RegistryError::Duplicate {
                module: key.0,
                symbol: key.1.to_string(),
            });
        }
        let index = self.handlers.len();
        self.handlers.push(ServiceDescriptor {
            module: key.0.clone(),
            symbol: key.1.to_string(),
            handler,
            status,
        });
        self.services.insert(key, index);
        Ok(())
    }

    /// Register a named service.
    ///
    /// # Panics
    ///
    /// If the name is invalid or already registered, so that one host module
    /// cannot silently replace another's service.
    pub fn register<F>(&mut self, module: &str, symbol: &str, handler: F)
    where
        F: Fn(&Win64Args, &mut CpuState, &mut GuestMemory) -> Result<u64, ExitReason>
            + Send
            + Sync
            + 'static,
    {
        self.register_with_status(module, symbol, ServiceStatus::Implemented, handler);
    }

    /// Describe the canonical binding behind an import without calling it.
    pub fn binding(&self, module: &str, symbol: &str) -> Option<&ServiceDescriptor> {
        self.service_index(module, symbol)
            .map(|index| &self.handlers[index])
    }

    /// Iterate all canonical registered (module, symbol, status) tuples.
    ///
    /// Each entry is returned exactly once regardless of how many API-set alias
    /// keys point at the same underlying handler.
    pub fn iter_registered(&self) -> impl Iterator<Item = (&str, &str, ServiceStatus)> {
        self.handlers
            .iter()
            .map(|d| (d.module.as_str(), d.symbol.as_str(), d.status))
    }

    /// Return (alias_module_lowercase, symbol) pairs that resolve to a canonical handler
    /// in a different module.
    pub fn iter_aliases(&self) -> impl Iterator<Item = (&str, &str, &str)> {
        // `services` maps (module_lc, symbol) -> handler_index.
        // An alias exists when module_lc != handlers[index].module.to_lowercase().
        self.services.iter().filter_map(|((mod_lc, name), &idx)| {
            let ImportName::Name(sym) = name else {
                return None;
            };
            let canonical_mod = self.handlers[idx].module.to_ascii_lowercase();
            if *mod_lc != canonical_mod {
                Some((
                    mod_lc.as_str(),
                    sym.as_str(),
                    self.handlers[idx].module.as_str(),
                ))
            } else {
                None
            }
        })
    }

    pub fn set_status(&mut self, module: &str, symbol: &str, status: ServiceStatus) {
        let index = self
            .service_index(module, symbol)
            .expect("registered service");
        self.handlers[index].status = status;
    }

    /// Register a named service with an explicit implementation status.
    ///
    /// # Panics
    ///
    /// As [`Self::register`].
    pub fn register_with_status<F>(
        &mut self,
        module: &str,
        symbol: &str,
        status: ServiceStatus,
        handler: F,
    ) where
        F: Fn(&Win64Args, &mut CpuState, &mut GuestMemory) -> Result<u64, ExitReason>
            + Send
            + Sync
            + 'static,
    {
        let name = ImportName::Name(symbol.to_owned());
        if let Err(error) = self.try_register(module, name, status, Box::new(handler)) {
            panic!("{error}");
        }
    }

    /// Make `module` resolve `symbol` to the service registered as
    /// `target!symbol`, sharing its trampoline.
    pub(crate) fn alias(&mut self, module: &str, target: &str, symbol: &str) {
        self.try_alias(module, target, symbol)
            .expect("valid built-in service alias");
    }

    pub fn try_alias(
        &mut self,
        module: &str,
        target: &str,
        symbol: &str,
    ) -> Result<(), RegistryError> {
        self.alias_key(module, target, symbol, ImportName::Name(symbol.to_owned()))
    }

    /// Publish a documented ordinal for a named export in its canonical module.
    pub fn try_ordinal_alias(
        &mut self,
        module: &str,
        symbol: &str,
        ordinal: u16,
    ) -> Result<(), RegistryError> {
        self.alias_key(module, module, symbol, ImportName::Ordinal(ordinal))
    }

    fn alias_key(
        &mut self,
        module: &str,
        target: &str,
        symbol: &str,
        key: ImportName,
    ) -> Result<(), RegistryError> {
        Self::check_key(module, &key)?;
        let index =
            self.service_index(target, symbol)
                .ok_or_else(|| RegistryError::UnknownTarget {
                    module: target.to_owned(),
                    symbol: symbol.to_owned(),
                })?;
        let previous = *self
            .services
            .entry((module.to_ascii_lowercase(), key.clone()))
            .or_insert(index);
        if previous == index {
            Ok(())
        } else {
            Err(RegistryError::Duplicate {
                module: module.to_owned(),
                symbol: key.to_string(),
            })
        }
    }

    /// Bind the PE's Import Address Table (IAT) in guest memory to synthetic trampoline VAs.
    pub fn bind_imports(
        &mut self,
        pe: &ParsedPe,
        bytes: &[u8],
        loaded_base: u64,
        memory: &mut GuestMemory,
    ) -> Result<usize, ImportError> {
        let Some(directory) = find_import_directory(bytes) else {
            return Ok(0);
        };
        let (entries, error) = parse_imports(bytes, directory, |rva| pe.rva_to_file_offset(rva));
        if let Some(error) = error {
            return Err(ImportError::Table(error));
        }

        // Resolve everything first: a refused image must leave memory unbound.
        let mut slots = Vec::with_capacity(entries.len());
        for entry in entries {
            let Some(index) = self.index(&entry.module, &entry.name) else {
                return Err(ImportError::UnresolvedImport {
                    module: entry.module,
                    symbol: entry.name.to_string(),
                });
            };
            let address = loaded_base
                .checked_add(u64::from(entry.iat_rva))
                .ok_or(MemoryError::Overflow)?;
            // The loader writes slots regardless of page permissions, so only
            // mapping is checked here.
            memory.read_u64_bypass(address)?;
            slots.push((address, index));
        }
        for &(address, index) in &slots {
            let trampoline = IMPORT_TRAMPOLINE_BASE_VA + (index as u64) * IMPORT_TRAMPOLINE_STRIDE;
            memory.write_u64_bypass(address, trampoline)?;
            self.trampoline_to_service.insert(trampoline, index);
        }
        Ok(slots.len())
    }

    pub(crate) fn register_callback_dispatch(
        &mut self,
        module: &str,
        symbol: &str,
        handler: impl Fn(&Win64Args, &GuestMemory) -> Result<Option<GuestCallback>, ExitReason>
        + Send
        + Sync
        + 'static,
    ) {
        self.register_with_status(module, symbol, ServiceStatus::Partial, |_, _, _| {
            Err(ExitReason::IllegalInstruction(0xff))
        });
        let index = self
            .service_index(module, symbol)
            .expect("registered dispatcher");
        self.callback_dispatchers.insert(index, Box::new(handler));
    }

    /// Check if target RIP matches a synthetic service trampoline.
    pub fn is_service_trampoline(&self, rip: u64) -> bool {
        rip == CALLBACK_RETURN
            || self.trampoline_to_service.contains_key(&rip)
            || self.dynamic_trampolines.get(rip).is_some()
    }

    /// Dispatch a service call when guest RIP hits a service trampoline.
    pub fn dispatch(
        &self,
        rip: u64,
        cpu: &mut CpuState,
        memory: &mut GuestMemory,
    ) -> Result<(), ExitReason> {
        if rip == CALLBACK_RETURN {
            let key = (cpu.gs_base, cpu.get(Register::Rsp));
            let frame = memory
                .callback_frames
                .remove(&key)
                .ok_or(ExitReason::IllegalInstruction(0xff))?;
            if Nonvolatile::of(cpu) != frame.preserved {
                return Err(ExitReason::ServiceAbiViolation);
            }
            cpu.set(Register::Rax, 1);
            cpu.rip = frame.resume;
            return Ok(());
        }
        let service_idx = self
            .trampoline_to_service
            .get(&rip)
            .copied()
            .or_else(|| self.dynamic_trampolines.get(rip))
            .ok_or(ExitReason::IllegalInstruction(0xFF))?;

        let descriptor = &self.handlers[service_idx];

        // Read the return address first, so a call that cannot return has no effect.
        let rsp = cpu.get(Register::Rsp);
        let ret_addr = memory.read_u64(rsp).map_err(ExitReason::MemoryFault)?;
        let after = rsp
            .checked_add(8)
            .ok_or(ExitReason::MemoryFault(MemoryError::Overflow))?;

        let args = Win64Args::new(cpu);
        let preserved = Nonvolatile::of(cpu);
        if let Some(dispatcher) = self.callback_dispatchers.get(&service_idx) {
            let key = (cpu.gs_base, after);
            if memory.callback_frames.len() >= MAX_CALLBACK_DEPTH
                || memory.callback_frames.contains_key(&key)
            {
                return Err(ExitReason::ServiceAbiViolation);
            }
            memory
                .check_write(rsp, 8)
                .map_err(ExitReason::MemoryFault)?;
            if let Some(callback) = dispatcher(&args, memory)? {
                memory
                    .write_u64(rsp, CALLBACK_RETURN)
                    .map_err(ExitReason::MemoryFault)?;
                let mut preserved = preserved;
                preserved.gpr[4] = after;
                memory.callback_frames.insert(
                    key,
                    CallbackFrame {
                        resume: ret_addr,
                        preserved,
                    },
                );
                cpu.set(Register::Rcx, callback.context);
                cpu.set(Register::Rdx, u64::from(callback.cancelled));
                cpu.rip = callback.entry;
                return Ok(());
            }
            cpu.set(Register::Rax, 0);
            cpu.set(Register::Rsp, after);
            cpu.rip = ret_addr;
            return Ok(());
        }
        let value = (descriptor.handler)(&args, cpu, memory)?;
        if Nonvolatile::of(cpu) != preserved {
            return Err(ExitReason::ServiceAbiViolation);
        }

        cpu.set(Register::Rax, value);
        cpu.set(Register::Rsp, after);
        cpu.rip = ret_addr;
        Ok(())
    }
}

/// Registers a Win64 callee must preserve (Microsoft x64 calling convention).
#[derive(Clone, PartialEq, Eq)]
struct Nonvolatile {
    gpr: [u64; 9],
    xmm: [u128; 10],
}

impl Nonvolatile {
    fn of(cpu: &CpuState) -> Self {
        use Register::{R12, R13, R14, R15, Rbp, Rbx, Rdi, Rsi, Rsp};
        let mut xmm = [0; 10];
        xmm.copy_from_slice(&cpu.xmm[6..]);
        Self {
            gpr: [Rbx, Rbp, Rdi, Rsi, Rsp, R12, R13, R14, R15].map(|register| cpu.get(register)),
            xmm,
        }
    }
}

/// RVA of the import directory, if the image has one.
pub(crate) fn find_import_directory(bytes: &[u8]) -> Option<u32> {
    let field = |offset: usize, len: usize| bytes.get(offset..offset.checked_add(len)?);
    let u32_at = |offset| Some(u32::from_le_bytes(field(offset, 4)?.try_into().ok()?));
    let coff = (u32_at(0x3C)? as usize).checked_add(4)?;
    let optional = coff.checked_add(20)?;
    let optional_len = u16::from_le_bytes(field(coff + 16, 2)?.try_into().ok()?);
    // The import directory is data directory 1, at offset 120.
    if optional_len < 128 {
        return None;
    }
    let rva = u32_at(optional.checked_add(120)?)?;
    let size = u32_at(optional.checked_add(124)?)?;
    (rva != 0 && size != 0).then_some(rva)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::MemoryPermissions;

    #[test]
    fn win64_arguments_follow_register_and_stack_layout() {
        let args = Win64Args {
            rcx: 11,
            rdx: 12,
            r8: 13,
            r9: 14,
            rsp: 0x1000,
        };
        let mut memory = GuestMemory::new();
        memory
            .map_region(0x1000, 56, MemoryPermissions::READ_WRITE)
            .unwrap();
        memory.write_u64(0x1028, 15).unwrap();
        memory.write_u64(0x1030, 16).unwrap();
        for (index, expected) in (11..=16).enumerate() {
            assert_eq!(args.get(index, &memory), Ok(expected));
        }
        assert_eq!(
            args.get(6, &memory),
            Err(MemoryError::UnmappedAddress(0x1038))
        );
    }

    #[test]
    fn win64_stack_arguments_reject_overflow_in_offsets_and_addresses() {
        let mut args = Win64Args {
            rcx: 0,
            rdx: 0,
            r8: 0,
            r9: 0,
            rsp: 0x1000,
        };
        let memory = GuestMemory::new();
        assert_eq!(args.get(usize::MAX, &memory), Err(MemoryError::Overflow));
        assert_eq!(
            args.get(usize::MAX / 8 + 4, &memory),
            Err(MemoryError::Overflow)
        );
        args.rsp = u64::MAX - 39;
        assert_eq!(args.get(4, &memory), Err(MemoryError::Overflow));
    }
}
