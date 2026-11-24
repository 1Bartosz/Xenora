// SPDX-License-Identifier: GPL-3.0-or-later

//! Unnamed event objects and waits that can resolve without a guest scheduler.

use super::{
    ERROR_ACCESS_DENIED, ERROR_INVALID_HANDLE, ERROR_INVALID_PARAMETER, ERROR_NOT_ENOUGH_MEMORY,
    ERROR_NOT_SUPPORTED, GuestObject, KERNEL32, ServicesState,
};
use crate::cpu::ExitReason;
use crate::imports::{ImportRegistry, ServiceStatus};
use crate::memory::MemoryError;
use std::sync::{Arc, Mutex};

const SYNCH_API: &str = "api-ms-win-core-synch-l1-1-0.dll";
const EVENT_MODIFY_STATE: u32 = 0x0002;
const SYNCHRONIZE: u32 = 0x0010_0000;
const EVENT_ALL_ACCESS: u32 = 0x001f_0003;
const WAIT_OBJECT_0: u64 = 0;
const WAIT_TIMEOUT: u64 = 258;
const WAIT_FAILED: u64 = u32::MAX as u64;

pub(super) struct Event {
    manual_reset: bool,
    signaled: bool,
}

pub(super) fn register(registry: &mut ImportRegistry, state: &Arc<ServicesState>) {
    for symbol in ["CreateEventA", "CreateEventW"] {
        let state = Arc::clone(state);
        registry.register(KERNEL32, symbol, move |args, _, mem| {
            let attributes = args.get(0, mem).map_err(ExitReason::MemoryFault)?;
            let manual_reset = args.get(1, mem).map_err(ExitReason::MemoryFault)? as u32 != 0;
            let signaled = args.get(2, mem).map_err(ExitReason::MemoryFault)? as u32 != 0;
            let name = args.get(3, mem).map_err(ExitReason::MemoryFault)?;
            if attributes != 0 || name != 0 {
                return state.fail(ERROR_NOT_SUPPORTED, 0);
            }
            let mut objects = state.objects.lock().expect("guest object state");
            let Some(handle) = objects.next_handle() else {
                return state.fail(ERROR_NOT_ENOUGH_MEMORY, 0);
            };
            objects.insert_handle(
                handle,
                GuestObject::Event {
                    state: Arc::new(Mutex::new(Event {
                        manual_reset,
                        signaled,
                    })),
                    access: EVENT_ALL_ACCESS,
                },
            );
            Ok(handle)
        });
    }

    for (symbol, signaled) in [("SetEvent", true), ("ResetEvent", false)] {
        let state = Arc::clone(state);
        registry.register(KERNEL32, symbol, move |args, _, mem| {
            let handle = args.get(0, mem).map_err(ExitReason::MemoryFault)?;
            let objects = state.objects.lock().expect("guest object state");
            let Some(GuestObject::Event {
                state: event,
                access,
            }) = objects.handles.get(&handle)
            else {
                return state.fail(ERROR_INVALID_HANDLE, 0);
            };
            if access & EVENT_MODIFY_STATE == 0 {
                return state.fail(ERROR_ACCESS_DENIED, 0);
            }
            event.lock().expect("guest event state").signaled = signaled;
            Ok(1)
        });
    }

    let single = Arc::clone(state);
    registry.register(KERNEL32, "WaitForSingleObject", move |args, _, mem| {
        let handle = args.get(0, mem).map_err(ExitReason::MemoryFault)?;
        let timeout = args.get(1, mem).map_err(ExitReason::MemoryFault)? as u32;
        wait_for_events(&single, &[handle], false, timeout)
    });

    let multiple = Arc::clone(state);
    registry.register(KERNEL32, "WaitForMultipleObjects", move |args, _, mem| {
        let count = args.get(0, mem).map_err(ExitReason::MemoryFault)? as u32;
        let address = args.get(1, mem).map_err(ExitReason::MemoryFault)?;
        let all = args.get(2, mem).map_err(ExitReason::MemoryFault)? as u32 != 0;
        let timeout = args.get(3, mem).map_err(ExitReason::MemoryFault)? as u32;
        if !(1..=64).contains(&count) || address == 0 {
            return multiple.fail(ERROR_INVALID_PARAMETER, WAIT_FAILED);
        }
        let mut handles = [0; 64];
        let handles = &mut handles[..count as usize];
        for (index, handle) in handles.iter_mut().enumerate() {
            let slot = address
                .checked_add(index as u64 * 8)
                .ok_or(ExitReason::MemoryFault(MemoryError::Overflow))?;
            *handle = mem.read_u64(slot).map_err(ExitReason::MemoryFault)?;
        }
        wait_for_events(&multiple, handles, all, timeout)
    });

    for symbol in [
        "CreateEventA",
        "CreateEventW",
        "SetEvent",
        "ResetEvent",
        "WaitForSingleObject",
        "WaitForMultipleObjects",
    ] {
        registry.alias(SYNCH_API, KERNEL32, symbol);
        registry.set_status(KERNEL32, symbol, ServiceStatus::Partial);
    }
    registry.alias(
        "api-ms-win-core-synch-l1-2-1.dll",
        KERNEL32,
        "WaitForMultipleObjects",
    );
}

fn wait_for_events(
    state: &ServicesState,
    handles: &[u64],
    all: bool,
    timeout: u32,
) -> Result<u64, ExitReason> {
    // The object table remains locked through validation and consumption.
    let objects = state.objects.lock().expect("guest object state");
    let mut events: Vec<&Arc<Mutex<Event>>> = Vec::with_capacity(handles.len());
    for (index, handle) in handles.iter().enumerate() {
        if handles[..index].contains(handle) {
            return state.fail(ERROR_INVALID_PARAMETER, WAIT_FAILED);
        }
        match objects.handles.get(handle) {
            Some(GuestObject::Event {
                state: event,
                access,
            }) => {
                if access & SYNCHRONIZE == 0 {
                    return state.fail(ERROR_ACCESS_DENIED, WAIT_FAILED);
                }
                if all && events.iter().any(|existing| Arc::ptr_eq(existing, event)) {
                    return state.fail(ERROR_INVALID_PARAMETER, WAIT_FAILED);
                }
                events.push(event);
            }
            Some(_) => return state.fail(ERROR_NOT_SUPPORTED, WAIT_FAILED),
            None if matches!(*handle, super::CURRENT_THREAD | super::CURRENT_PROCESS) => {
                return state.fail(ERROR_NOT_SUPPORTED, WAIT_FAILED);
            }
            None => return state.fail(ERROR_INVALID_HANDLE, WAIT_FAILED),
        }
    }
    let mut ready = Vec::with_capacity(handles.len());
    for (index, event) in events.into_iter().enumerate() {
        if event.lock().expect("guest event state").signaled {
            ready.push((index, event));
            if !all {
                break;
            }
        }
    }
    if ready.is_empty() || (all && ready.len() != handles.len()) {
        return if timeout == 0 {
            Ok(WAIT_TIMEOUT)
        } else {
            state.fail(ERROR_NOT_SUPPORTED, WAIT_FAILED)
        };
    }
    let first = ready[0].0;
    for (_, event) in ready {
        let mut event = event.lock().expect("guest event state");
        event.signaled = event.manual_reset;
    }
    Ok(WAIT_OBJECT_0 + first as u64)
}
