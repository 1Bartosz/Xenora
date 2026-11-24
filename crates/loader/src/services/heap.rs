// SPDX-License-Identifier: GPL-3.0-or-later

//! Default heap services backed by owned guest memory regions.

use super::{
    ERROR_INVALID_ADDRESS, ERROR_INVALID_HANDLE, ERROR_NOT_SUPPORTED, KERNEL32, ServicesState,
};
use crate::cpu::ExitReason;
use crate::imports::{ImportRegistry, ServiceStatus};
use std::sync::Arc;

const PROCESS_HEAP: u64 = u64::MAX - 2;
const HEAP_ZERO_MEMORY: u32 = 8;
const HEAP_REALLOC_IN_PLACE_ONLY: u32 = 16;
const HEAP_API: &str = "api-ms-win-core-heap-l1-1-0.dll";

pub(super) fn register(registry: &mut ImportRegistry, state: &Arc<ServicesState>) {
    registry.register(KERNEL32, "GetProcessHeap", |_, _, _| Ok(PROCESS_HEAP));

    registry.register(KERNEL32, "HeapAlloc", |args, _, memory| {
        let handle = args.get(0, memory).map_err(ExitReason::MemoryFault)?;
        let flags = args.get(1, memory).map_err(ExitReason::MemoryFault)? as u32;
        let size = args.get(2, memory).map_err(ExitReason::MemoryFault)?;
        if handle != PROCESS_HEAP || flags & !HEAP_ZERO_MEMORY != 0 {
            return Ok(0);
        }
        Ok(memory
            .map_heap_allocation(size.max(1) as usize)
            .unwrap_or(0))
    });

    registry.register(KERNEL32, "HeapReAlloc", |args, _, memory| {
        let handle = args.get(0, memory).map_err(ExitReason::MemoryFault)?;
        let flags = args.get(1, memory).map_err(ExitReason::MemoryFault)? as u32;
        let address = args.get(2, memory).map_err(ExitReason::MemoryFault)?;
        let size = args.get(3, memory).map_err(ExitReason::MemoryFault)?;
        if handle != PROCESS_HEAP || flags & !(HEAP_ZERO_MEMORY | HEAP_REALLOC_IN_PLACE_ONLY) != 0 {
            return Ok(0);
        }
        let in_place_only = flags & HEAP_REALLOC_IN_PLACE_ONLY != 0;
        Ok(memory
            .resize_heap_allocation(address, size.max(1) as usize, in_place_only)
            .unwrap_or(0))
    });

    registry.register(KERNEL32, "HeapSize", |args, _, memory| {
        let handle = args.get(0, memory).map_err(ExitReason::MemoryFault)?;
        let flags = args.get(1, memory).map_err(ExitReason::MemoryFault)? as u32;
        let address = args.get(2, memory).map_err(ExitReason::MemoryFault)?;
        if handle != PROCESS_HEAP || flags != 0 {
            return Ok(u64::MAX);
        }
        Ok(memory
            .heap_allocation_size(address)
            .map_or(u64::MAX, |size| size as u64))
    });

    let state = Arc::clone(state);
    registry.register(KERNEL32, "HeapFree", move |args, _, memory| {
        let handle = args.get(0, memory).map_err(ExitReason::MemoryFault)?;
        let flags = args.get(1, memory).map_err(ExitReason::MemoryFault)? as u32;
        let address = args.get(2, memory).map_err(ExitReason::MemoryFault)?;
        if handle != PROCESS_HEAP {
            return state.fail(ERROR_INVALID_HANDLE, 0);
        }
        if flags != 0 {
            return state.fail(ERROR_NOT_SUPPORTED, 0);
        }
        if address != 0 && !memory.release_heap_allocation(address) {
            return state.fail(ERROR_INVALID_ADDRESS, 0);
        }
        Ok(1)
    });

    for symbol in [
        "GetProcessHeap",
        "HeapAlloc",
        "HeapReAlloc",
        "HeapSize",
        "HeapFree",
    ] {
        registry.alias(HEAP_API, KERNEL32, symbol);
        registry.set_status(KERNEL32, symbol, ServiceStatus::Partial);
    }
}
