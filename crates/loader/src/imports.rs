// SPDX-License-Identifier: GPL-3.0-or-later

//! Guest Import Table Binder and Host Service Dispatch Boundary for Xenora X15.
//!
//! Maps guest PE IAT (Import Address Table) entries to synthetic dispatch thunks,
//! validating the Windows x64 ABI calling convention and dispatching calls to typed
//! guest services without exposing raw host function pointers.

use crate::cpu::{CpuState, ExitReason, Register};
use crate::memory::{GuestMemory, MemoryError};
use crate::pe::ParsedPe;
use std::collections::HashMap;
use std::fmt;

/// Base virtual address range reserved for synthetic guest import dispatch trampolines.
pub const IMPORT_TRAMPOLINE_BASE_VA: u64 = 0x0000_0007_F000_0000;
pub const IMPORT_TRAMPOLINE_STRIDE: u64 = 16;

/// Typed error resulting from import resolution or service dispatch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ImportError {
    MalformedDirectory,
    DirectoryOutOfBounds { rva: u32 },
    UnresolvedImport { module: String, symbol: String },
    Memory(MemoryError),
}

impl fmt::Display for ImportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MalformedDirectory => write!(f, "malformed PE import directory"),
            Self::DirectoryOutOfBounds { rva } => {
                write!(f, "import directory at RVA 0x{rva:08x} is out of bounds")
            }
            Self::UnresolvedImport { module, symbol } => {
                write!(f, "unresolved import dependency: {module}!{symbol}")
            }
            Self::Memory(e) => write!(f, "guest memory error binding imports: {e}"),
        }
    }
}

impl std::error::Error for ImportError {}

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
                let stack_offset = 40 + ((n - 4) as u64) * 8;
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

/// Registry for resolving and binding import symbols to guest trampoline VAs.
pub struct ImportRegistry {
    services: HashMap<(String, String), usize>, // (module_lowercase, symbol) -> service_index
    handlers: Vec<ServiceDescriptor>,
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
            trampoline_to_service: HashMap::new(),
        }
    }

    fn service_index(&self, module: &str, symbol: &str) -> Option<usize> {
        self.services
            .get(&(module.to_ascii_lowercase(), symbol.to_owned()))
            .copied()
    }

    /// Check whether the exact import key has a binding, including stub handlers.
    pub fn has_binding(&self, module: &str, symbol: &str) -> bool {
        self.service_index(module, symbol).is_some()
    }

    /// Register a host-provided guest service.
    pub fn register<F>(&mut self, module: &str, symbol: &str, handler: F)
    where
        F: Fn(&Win64Args, &mut CpuState, &mut GuestMemory) -> Result<u64, ExitReason>
            + Send
            + Sync
            + 'static,
    {
        let mod_key = module.to_ascii_lowercase();
        let sym_key = symbol.to_string();
        let index = self.handlers.len();
        self.handlers.push(ServiceDescriptor {
            module: mod_key.clone(),
            symbol: sym_key.clone(),
            handler: Box::new(handler),
            status: ServiceStatus::Implemented,
        });
        self.services.insert((mod_key, sym_key), index);
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
        self.services.iter().filter_map(|((mod_lc, sym), &idx)| {
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

    pub(crate) fn set_status(&mut self, module: &str, symbol: &str, status: ServiceStatus) {
        let index = self
            .service_index(module, symbol)
            .expect("registered service");
        self.handlers[index].status = status;
    }

    /// Point an API-set import at an existing handler and its trampoline.
    pub(crate) fn alias(&mut self, module: &str, target: &str, symbol: &str) {
        let index = self
            .service_index(target, symbol)
            .expect("registered alias target");
        self.services
            .insert((module.to_ascii_lowercase(), symbol.to_owned()), index);
    }

    /// Bind the PE's Import Address Table (IAT) in guest memory to synthetic trampoline VAs.
    pub fn bind_imports(
        &mut self,
        pe: &ParsedPe,
        bytes: &[u8],
        loaded_base: u64,
        memory: &mut GuestMemory,
    ) -> Result<usize, ImportError> {
        let import_dir = find_import_directory(bytes)?;
        let Some((import_rva, import_size)) = import_dir else {
            return Ok(0);
        };
        if import_size == 0 {
            return Ok(0);
        }

        let mut desc_file_off = pe
            .rva_to_file_offset(import_rva)
            .ok_or(ImportError::DirectoryOutOfBounds { rva: import_rva })?;

        let mut bound_count = 0;

        while desc_file_off + 20 <= bytes.len() {
            let oft =
                u32::from_le_bytes(bytes[desc_file_off..desc_file_off + 4].try_into().unwrap());
            let name_rva = u32::from_le_bytes(
                bytes[desc_file_off + 12..desc_file_off + 16]
                    .try_into()
                    .unwrap(),
            );
            let ft = u32::from_le_bytes(
                bytes[desc_file_off + 16..desc_file_off + 20]
                    .try_into()
                    .unwrap(),
            );

            if oft == 0 && name_rva == 0 && ft == 0 {
                break;
            }
            desc_file_off += 20;

            let mod_name = if let Some(no) = pe.rva_to_file_offset(name_rva) {
                let mut end = no;
                while end < bytes.len() && bytes[end] != 0 {
                    end += 1;
                }
                String::from_utf8_lossy(&bytes[no..end]).into_owned()
            } else {
                "UNKNOWN.dll".to_string()
            };

            let thunk_rva = if oft != 0 { oft } else { ft };
            let iat_base_rva = ft;

            let Some(mut thunk_off) = pe.rva_to_file_offset(thunk_rva) else {
                continue;
            };

            let mut idx = 0u32;
            while thunk_off + 8 <= bytes.len() {
                let val = u64::from_le_bytes(bytes[thunk_off..thunk_off + 8].try_into().unwrap());
                if val == 0 {
                    break;
                }
                thunk_off += 8;
                let iat_rva = iat_base_rva + idx * 8;
                idx += 1;

                let sym_name = if (val & (1 << 63)) != 0 {
                    format!("Ordinal#{}", val & 0xFFFF)
                } else {
                    let hint_rva = (val & 0x7FFF_FFFF) as u32;
                    if let Some(ho) = pe.rva_to_file_offset(hint_rva) {
                        let str_start = ho + 2;
                        let mut str_end = str_start;
                        while str_end < bytes.len() && bytes[str_end] != 0 {
                            str_end += 1;
                        }
                        String::from_utf8_lossy(&bytes[str_start..str_end]).into_owned()
                    } else {
                        "UNKNOWN_SYM".to_string()
                    }
                };

                let Some(service_idx) = self.service_index(&mod_name, &sym_name) else {
                    return Err(ImportError::UnresolvedImport {
                        module: mod_name,
                        symbol: sym_name,
                    });
                };

                // Assign a deterministic synthetic trampoline VA
                let trampoline_va =
                    IMPORT_TRAMPOLINE_BASE_VA + (service_idx as u64) * IMPORT_TRAMPOLINE_STRIDE;
                self.trampoline_to_service
                    .insert(trampoline_va, service_idx);

                // Write trampoline_va into guest IAT entry at (loaded_base + iat_rva)
                let iat_va = loaded_base + iat_rva as u64;
                memory.write_u64_bypass(iat_va, trampoline_va)?;
                bound_count += 1;
            }
        }

        Ok(bound_count)
    }

    /// Check if target RIP matches a synthetic service trampoline.
    pub fn is_service_trampoline(&self, rip: u64) -> bool {
        self.trampoline_to_service.contains_key(&rip)
    }

    /// Dispatch a service call when guest RIP hits a service trampoline.
    pub fn dispatch(
        &self,
        rip: u64,
        cpu: &mut CpuState,
        memory: &mut GuestMemory,
    ) -> Result<(), ExitReason> {
        let service_idx = *self
            .trampoline_to_service
            .get(&rip)
            .ok_or(ExitReason::IllegalInstruction(0xFF))?;

        let descriptor = &self.handlers[service_idx];

        // 1. Extract arguments according to Win64 convention
        let args = Win64Args::new(cpu);

        // 2. Execute service handler
        let ret_val = (descriptor.handler)(&args, cpu, memory)?;

        // 3. Store return value into RAX
        cpu.set(Register::Rax, ret_val);

        // 4. Emulate RET instruction: pop return address from guest stack into RIP
        let rsp = cpu.get(Register::Rsp);
        let ret_addr = memory.read_u64(rsp).map_err(ExitReason::MemoryFault)?;
        cpu.set(Register::Rsp, rsp + 8);
        cpu.rip = ret_addr;

        Ok(())
    }
}

fn find_import_directory(bytes: &[u8]) -> Result<Option<(u32, u32)>, ImportError> {
    if bytes.len() < 0x40 {
        return Ok(None);
    }
    let e_lfanew = u32::from_le_bytes(bytes[0x3C..0x40].try_into().unwrap()) as usize;
    let opt_offset = e_lfanew + 24;
    let coff_offset = e_lfanew + 4;
    if bytes.len() < coff_offset + 18 {
        return Ok(None);
    }
    let size_of_opt = u16::from_le_bytes(
        bytes[coff_offset + 16..coff_offset + 18]
            .try_into()
            .unwrap(),
    ) as usize;

    // Import directory is directory index 1 (offset 112 + 1 * 8 = 120 in Optional Header)
    if size_of_opt < 128 || bytes.len() < opt_offset + 128 {
        return Ok(None);
    }

    let import_rva = u32::from_le_bytes(
        bytes[opt_offset + 120..opt_offset + 124]
            .try_into()
            .unwrap(),
    );
    let import_size = u32::from_le_bytes(
        bytes[opt_offset + 124..opt_offset + 128]
            .try_into()
            .unwrap(),
    );

    if import_rva == 0 || import_size == 0 {
        Ok(None)
    } else {
        Ok(Some((import_rva, import_size)))
    }
}
