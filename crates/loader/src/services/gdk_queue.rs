// SPDX-License-Identifier: GPL-3.0-or-later

//! Handle ownership for the documented manual GDK task queue subset.

use super::ServicesState;
use super::{ERROR_INVALID_HANDLE, ERROR_INVALID_PARAMETER, ERROR_NOT_SUPPORTED};
use crate::cpu::ExitReason;
use crate::imports::{GuestCallback, ImportRegistry, ServiceStatus};
use crate::memory::GuestMemory;
use crate::runtime::HResult;
use std::collections::{BTreeMap, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant};

const MODULE: &str = "xgameruntime.dll";
const MAX_HANDLES: usize = 64;
const MAX_DISPATCH_WAIT_MS: u32 = 1_000;
const MAX_CALLBACKS_PER_PORT: usize = 32;
const ERROR_OUTOFMEMORY: u32 = 14;
const MANUAL: u32 = 0;

#[derive(Default)]
pub(super) struct QueueHandles {
    last_id: u64,
    queues: BTreeMap<u64, [u64; 2]>,
    pending: BTreeMap<u64, VecDeque<ScheduledCallback>>,
}

struct ScheduledCallback {
    ready_at: Instant,
    callback: GuestCallback,
}

impl QueueHandles {
    fn open(
        &mut self,
        ports: Option<[u64; 2]>,
        output: u64,
        memory: &mut GuestMemory,
    ) -> Result<(), u32> {
        if output == 0 {
            return Err(ERROR_INVALID_PARAMETER);
        }
        if self.queues.len() == MAX_HANDLES {
            return Err(ERROR_OUTOFMEMORY);
        }
        let next = self
            .last_id
            .checked_add(1)
            .filter(|id| *id < (1 << 48))
            .ok_or(ERROR_OUTOFMEMORY)?;
        let handle = 0x5144_0000_0000_0000 | next;
        memory
            .write_u64(output, handle)
            .map_err(|_| ERROR_INVALID_PARAMETER)?;
        self.last_id = next;
        self.queues.insert(
            handle,
            ports.unwrap_or([0x5157_0000_0000_0000 | next, 0x5143_0000_0000_0000 | next]),
        );
        Ok(())
    }

    fn close(&mut self, handle: u64) {
        if let Some(ports) = self.queues.remove(&handle) {
            for port in ports {
                if !self.queues.values().any(|live| live.contains(&port)) {
                    self.pending.remove(&port);
                }
            }
        }
    }

    fn submit(
        &mut self,
        handle: u64,
        selector: u32,
        callback: ScheduledCallback,
    ) -> Result<(), u32> {
        let ports = self.queues.get(&handle).ok_or(ERROR_INVALID_HANDLE)?;
        let port = ports
            .get(selector as usize)
            .ok_or(ERROR_INVALID_PARAMETER)?;
        let queue = self.pending.entry(*port).or_default();
        if queue.len() >= MAX_CALLBACKS_PER_PORT {
            return Err(ERROR_OUTOFMEMORY);
        }
        let index = queue.partition_point(|item| item.ready_at <= callback.ready_at);
        queue.insert(index, callback);
        Ok(())
    }
}

fn guest_hresult(result: Result<(), u32>) -> u64 {
    match result {
        Ok(()) => HResult::OK.guest_return(),
        Err(error) => HResult::from_win32(error).guest_return(),
    }
}

pub(super) fn register(registry: &mut ImportRegistry, state: &Arc<ServicesState>) {
    let create = Arc::clone(state);
    registry.register_with_status(
        MODULE,
        "XTaskQueueCreate",
        ServiceStatus::Partial,
        move |args, _, memory| {
            if args.rcx as u32 != MANUAL || args.rdx as u32 != MANUAL {
                return Ok(guest_hresult(Err(ERROR_NOT_SUPPORTED)));
            }
            let mut queues = create.gdk_queues.lock().expect("GDK queue handle table");
            Ok(guest_hresult(queues.open(None, args.r8, memory)))
        },
    );

    let duplicate = Arc::clone(state);
    registry.register_with_status(
        MODULE,
        "XTaskQueueDuplicateHandle",
        ServiceStatus::Partial,
        move |args, _, memory| {
            let mut queues = duplicate.gdk_queues.lock().expect("GDK queue handle table");
            let Some(&object) = queues.queues.get(&args.rcx) else {
                return Ok(guest_hresult(Err(ERROR_INVALID_HANDLE)));
            };
            Ok(guest_hresult(queues.open(Some(object), args.rdx, memory)))
        },
    );

    let close = Arc::clone(state);
    registry.register_with_status(
        MODULE,
        "XTaskQueueCloseHandle",
        ServiceStatus::Partial,
        move |args, _, _| {
            close
                .gdk_queues
                .lock()
                .expect("GDK queue handle table")
                .close(args.rcx);
            close.gdk_queue_changed.notify_all();
            Ok(0)
        },
    );

    let get = Arc::clone(state);
    registry.register_with_status(
        MODULE,
        "XTaskQueueGetPort",
        ServiceStatus::Partial,
        move |args, _, memory| {
            let result = (|| {
                let queues = get.gdk_queues.lock().expect("GDK queue handle table");
                let ports = queues.queues.get(&args.rcx).ok_or(ERROR_INVALID_HANDLE)?;
                let port = ports
                    .get(args.rdx as u32 as usize)
                    .ok_or(ERROR_INVALID_PARAMETER)?;
                if args.r8 == 0 {
                    return Err(ERROR_INVALID_PARAMETER);
                }
                memory
                    .write_u64(args.r8, *port)
                    .map_err(|_| ERROR_INVALID_PARAMETER)
            })();
            Ok(guest_hresult(result))
        },
    );

    let composite = Arc::clone(state);
    registry.register_with_status(
        MODULE,
        "XTaskQueueCreateComposite",
        ServiceStatus::Partial,
        move |args, _, memory| {
            let mut queues = composite.gdk_queues.lock().expect("GDK queue handle table");
            let ports = [args.rcx, args.rdx];
            if ports
                .iter()
                .any(|port| !queues.queues.values().any(|live| live.contains(port)))
            {
                return Ok(guest_hresult(Err(ERROR_INVALID_HANDLE)));
            }
            Ok(guest_hresult(queues.open(Some(ports), args.r8, memory)))
        },
    );

    for (symbol, delayed) in [
        ("XTaskQueueSubmitCallback", false),
        ("XTaskQueueSubmitDelayedCallback", true),
    ] {
        let submit = Arc::clone(state);
        registry.register_with_status(
            MODULE,
            symbol,
            ServiceStatus::Partial,
            move |args, _, memory| {
                let (delay, context, entry) = if delayed {
                    let Ok(entry) = args.get(4, memory) else {
                        return Ok(guest_hresult(Err(ERROR_INVALID_PARAMETER)));
                    };
                    (args.r8 as u32, args.r9, entry)
                } else {
                    (0, args.r8, args.r9)
                };
                if entry == 0 || memory.fetch_instruction_byte(entry).is_err() {
                    return Ok(guest_hresult(Err(ERROR_INVALID_PARAMETER)));
                }
                let mut queues = submit.gdk_queues.lock().expect("GDK queue handle table");
                let callback = ScheduledCallback {
                    ready_at: Instant::now() + Duration::from_millis(u64::from(delay)),
                    callback: GuestCallback { entry, context },
                };
                let result = queues.submit(args.rcx, args.rdx as u32, callback);
                if result.is_ok() {
                    submit.gdk_queue_changed.notify_all();
                }
                Ok(guest_hresult(result))
            },
        );
    }
    let dispatch = Arc::clone(state);
    registry.register_callback_dispatch(MODULE, "XTaskQueueDispatch", move |args, memory| {
        let timeout = args.r8 as u32;
        if timeout > MAX_DISPATCH_WAIT_MS {
            return Ok(None);
        }
        let deadline = Instant::now() + Duration::from_millis(u64::from(timeout));
        let mut queues = dispatch.gdk_queues.lock().expect("GDK queue handle table");
        loop {
            let Some(&port) = queues
                .queues
                .get(&args.rcx)
                .and_then(|ports| ports.get(args.rdx as u32 as usize))
            else {
                return Ok(None);
            };
            let now = Instant::now();
            let mut wake_at = deadline;
            if let Some(pending) = queues.pending.get_mut(&port)
                && let Some(item) = pending.front()
            {
                if item.ready_at <= now {
                    memory
                        .fetch_instruction_byte(item.callback.entry)
                        .map_err(ExitReason::MemoryFault)?;
                    return Ok(pending.pop_front().map(|item| item.callback));
                }
                wake_at = wake_at.min(item.ready_at);
            }
            let remaining = wake_at.saturating_duration_since(now);
            if remaining.is_zero() {
                return Ok(None);
            }
            (queues, _) = dispatch
                .gdk_queue_changed
                .wait_timeout(queues, remaining)
                .expect("GDK queue handle table");
        }
    });
}
