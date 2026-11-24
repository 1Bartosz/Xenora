// SPDX-License-Identifier: GPL-3.0-or-later

//! Unnamed event objects and waits that can resolve without a guest scheduler.

use super::{
    ERROR_ACCESS_DENIED, ERROR_INVALID_HANDLE, ERROR_NOT_ENOUGH_MEMORY, ERROR_NOT_SUPPORTED,
    GuestObject, KERNEL32, ServicesState,
};
use crate::cpu::ExitReason;
use crate::imports::{ImportRegistry, ServiceStatus};
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

    let state = Arc::clone(state);
    registry.register(KERNEL32, "WaitForSingleObject", move |args, _, mem| {
        let handle = args.get(0, mem).map_err(ExitReason::MemoryFault)?;
        let timeout = args.get(1, mem).map_err(ExitReason::MemoryFault)? as u32;
        let objects = state.objects.lock().expect("guest object state");
        let Some(GuestObject::Event {
            state: event,
            access,
        }) = objects.handles.get(&handle)
        else {
            if objects.handles.contains_key(&handle)
                || matches!(handle, super::CURRENT_THREAD | super::CURRENT_PROCESS)
            {
                return state.fail(ERROR_NOT_SUPPORTED, WAIT_FAILED);
            }
            return state.fail(ERROR_INVALID_HANDLE, WAIT_FAILED);
        };
        if access & SYNCHRONIZE == 0 {
            return state.fail(ERROR_ACCESS_DENIED, WAIT_FAILED);
        }
        let mut event = event.lock().expect("guest event state");
        if event.signaled {
            event.signaled = event.manual_reset;
            return Ok(WAIT_OBJECT_0);
        }
        if timeout == 0 {
            return Ok(WAIT_TIMEOUT);
        }
        state.fail(ERROR_NOT_SUPPORTED, WAIT_FAILED)
    });

    for symbol in [
        "CreateEventA",
        "CreateEventW",
        "SetEvent",
        "ResetEvent",
        "WaitForSingleObject",
    ] {
        registry.alias(SYNCH_API, KERNEL32, symbol);
        registry.set_status(KERNEL32, symbol, ServiceStatus::Partial);
    }
}
