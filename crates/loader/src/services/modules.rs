// SPDX-License-Identifier: GPL-3.0-or-later

//! Module and export lookup for built-in guest services.

use super::{ERROR_INVALID_HANDLE, KERNEL32, ServicesState};
use crate::cpu::ExitReason;
use crate::imports::{ImportRegistry, ServiceStatus};
use crate::memory::{GuestMemory, MemoryError};
use std::collections::BTreeMap;
use std::sync::{Arc, OnceLock};

const LOADER_API: &str = "api-ms-win-core-libraryloader-l1-2-0.dll";
const UCRT: &str = "ucrtbase.dll";
const MODULE_NOT_FOUND: u32 = 126;
const PROCEDURE_NOT_FOUND: u32 = 127;
const MODULE_BASE: u64 = 0x0000_0007_e000_0000;

#[derive(Default)]
struct Modules {
    names: BTreeMap<String, u64>,
    exports: BTreeMap<(u64, String), usize>,
}

fn read_name(
    memory: &GuestMemory,
    address: u64,
    wide: bool,
) -> Result<Option<String>, MemoryError> {
    let mut name = String::new();
    let width = if wide { 2 } else { 1 };
    for index in 0..260 {
        let slot = address
            .checked_add(index * width)
            .ok_or(MemoryError::Overflow)?;
        let unit = if wide {
            memory.read_u16(slot)?
        } else {
            u16::from(memory.read_u8(slot)?)
        };
        if unit == 0 {
            return Ok(Some(name));
        }
        if !(0x20..=0x7e).contains(&unit) {
            return Ok(None);
        }
        name.push(char::from(unit as u8));
    }
    Ok(None)
}

pub(super) fn register(registry: &mut ImportRegistry, state: &Arc<ServicesState>) {
    let modules = Arc::new(OnceLock::<Modules>::new());
    for (symbol, wide) in [("GetModuleHandleA", false), ("GetModuleHandleW", true)] {
        let state = Arc::clone(state);
        let modules = Arc::clone(&modules);
        registry.register(KERNEL32, symbol, move |args, _, memory| {
            if args.rcx == 0 {
                return state.fail(MODULE_NOT_FOUND, 0);
            }
            let Some(name) = read_name(memory, args.rcx, wide).map_err(ExitReason::MemoryFault)?
            else {
                return state.fail(MODULE_NOT_FOUND, 0);
            };
            let mut name = name.to_ascii_lowercase();
            if !name.contains('.') {
                name.push_str(".dll");
            }
            let modules = modules.get().expect("built-in module catalog");
            match modules.names.get(&name) {
                Some(&handle) => Ok(handle),
                None => state.fail(MODULE_NOT_FOUND, 0),
            }
        });
    }

    let lookup = Arc::clone(&modules);
    let trampolines = registry.dynamic_trampolines();
    let state = Arc::clone(state);
    registry.register(KERNEL32, "GetProcAddress", move |args, _, memory| {
        let modules = lookup.get().expect("built-in module catalog");
        if !matches!(args.rcx, MODULE_BASE | UCRT_HANDLE) {
            return state.fail(ERROR_INVALID_HANDLE, 0);
        }
        // Ordinal entries require an independently verified export catalog.
        if args.rdx <= u16::MAX as u64 {
            return state.fail(PROCEDURE_NOT_FOUND, 0);
        }
        let Some(symbol) = read_name(memory, args.rdx, false).map_err(ExitReason::MemoryFault)?
        else {
            return state.fail(PROCEDURE_NOT_FOUND, 0);
        };
        match modules.exports.get(&(args.rcx, symbol)) {
            Some(&index) => Ok(trampolines.expose(index)),
            None => state.fail(PROCEDURE_NOT_FOUND, 0),
        }
    });

    for symbol in ["GetModuleHandleA", "GetModuleHandleW", "GetProcAddress"] {
        registry.alias(LOADER_API, KERNEL32, symbol);
        registry.set_status(KERNEL32, symbol, ServiceStatus::Partial);
    }

    let mut catalog = Modules::default();
    for (module, handle) in [(KERNEL32, MODULE_BASE), (UCRT, UCRT_HANDLE)] {
        catalog.names.insert(module.to_owned(), handle);
        for (name, symbol, status) in registry.iter_registered() {
            if name == module
                && status != ServiceStatus::Stub
                && let Some(index) = registry.service_index(name, symbol)
            {
                catalog.exports.insert((handle, symbol.to_owned()), index);
            }
        }
        for (alias, _, canonical) in registry.iter_aliases() {
            if canonical == module {
                catalog.names.insert(alias.to_owned(), handle);
            }
        }
    }
    assert!(modules.set(catalog).is_ok());
}

const UCRT_HANDLE: u64 = MODULE_BASE + 0x10000;
