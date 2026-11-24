// SPDX-License-Identifier: GPL-3.0-or-later

//! Bounded synthetic C: drive with no host filesystem mapping.

use super::{
    ERROR_ACCESS_DENIED, ERROR_INVALID_HANDLE, ERROR_INVALID_PARAMETER, ERROR_NOACCESS,
    ERROR_NOT_ENOUGH_MEMORY, ERROR_NOT_SUPPORTED, GuestObject, KERNEL32, ServicesState,
};
use crate::cpu::ExitReason;
use crate::imports::{ImportRegistry, ServiceStatus};
use crate::memory::{GuestMemory, MemoryError};
use std::sync::Arc;

const FILE_API: &str = "api-ms-win-core-file-l1-1-0.dll";
const INVALID_HANDLE: u64 = u64::MAX;
const READ: u32 = 0x8000_0000;
const WRITE: u32 = 0x4000_0000;
const FILE_NOT_FOUND: u32 = 2;
const FILE_EXISTS: u32 = 80;
const ALREADY_EXISTS: u32 = 183;
const INVALID_NAME: u32 = 123;
const SHARING_VIOLATION: u32 = 32;
const DISK_FULL: u32 = 112;
const MAX_FILES: usize = 256;
const MAX_BYTES: usize = 16 * 1024 * 1024;
const MAX_TRANSFER: usize = 1024 * 1024;

pub(super) struct OpenFile {
    path: String,
    position: usize,
    access: u32,
    share: u32,
}

fn flat_guest_name(memory: &GuestMemory, address: u64) -> Result<Option<String>, MemoryError> {
    if address == 0 {
        return Ok(None);
    }
    let mut units = Vec::new();
    for index in 0..260 {
        let slot = address
            .checked_add(index * 2)
            .ok_or(MemoryError::Overflow)?;
        let unit = memory.read_u16(slot)?;
        if unit == 0 {
            let Ok(path) = String::from_utf16(&units) else {
                return Ok(None);
            };
            let path = path.replace('/', "\\").to_ascii_lowercase();
            let name = path.strip_prefix("c:\\").unwrap_or(&path);
            let device = name.split('.').next().unwrap_or_default();
            if name.is_empty()
                || name.ends_with([' ', '.'])
                || name
                    .chars()
                    .any(|c| c.is_control() || "\\/:*?\"<>|".contains(c))
                || matches!(device, "con" | "prn" | "aux" | "nul")
                || (device.len() == 4
                    && (device.starts_with("com") || device.starts_with("lpt"))
                    && device.as_bytes()[3].is_ascii_digit())
            {
                return Ok(None);
            }
            return Ok(Some(name.to_owned()));
        }
        units.push(unit);
    }
    Ok(None)
}

fn share_mask(access: u32) -> u32 {
    u32::from(access & READ != 0) | (u32::from(access & WRITE != 0) << 1)
}

pub(super) fn register(registry: &mut ImportRegistry, state: &Arc<ServicesState>) {
    let open = Arc::clone(state);
    registry.register(KERNEL32, "CreateFileW", move |args, _, memory| {
        let access = args.rdx as u32;
        let share = args.r8 as u32;
        let disposition = args.get(4, memory).map_err(ExitReason::MemoryFault)? as u32;
        let flags = args.get(5, memory).map_err(ExitReason::MemoryFault)? as u32;
        let template = args.get(6, memory).map_err(ExitReason::MemoryFault)?;
        if args.r9 != 0
            || template != 0
            || !matches!(flags, 0 | 0x80)
            || access & !(READ | WRITE) != 0
            || share & !3 != 0
        {
            return open.fail(ERROR_NOT_SUPPORTED, INVALID_HANDLE);
        }
        if !(1..=5).contains(&disposition) {
            return open.fail(ERROR_INVALID_PARAMETER, INVALID_HANDLE);
        }
        if matches!(disposition, 2 | 5) && access & WRITE == 0 {
            return open.fail(ERROR_ACCESS_DENIED, INVALID_HANDLE);
        }
        let Some(path) = flat_guest_name(memory, args.rcx).map_err(ExitReason::MemoryFault)? else {
            return open.fail(INVALID_NAME, INVALID_HANDLE);
        };
        let mut objects = open.objects.lock().expect("guest object state");
        let exists = objects.files.contains_key(&path);
        if exists && disposition == 1 {
            return open.fail(FILE_EXISTS, INVALID_HANDLE);
        }
        if !exists && matches!(disposition, 3 | 5) {
            return open.fail(FILE_NOT_FOUND, INVALID_HANDLE);
        }
        for object in objects.handles.values() {
            if let GuestObject::File(file) = object
                && file.path == path
                && (share_mask(access) & !file.share != 0 || share_mask(file.access) & !share != 0)
            {
                return open.fail(SHARING_VIOLATION, INVALID_HANDLE);
            }
        }
        let Some(handle) = objects.next_handle() else {
            return open.fail(ERROR_NOT_ENOUGH_MEMORY, INVALID_HANDLE);
        };
        if !exists {
            if objects.files.len() >= MAX_FILES {
                return open.fail(DISK_FULL, INVALID_HANDLE);
            }
            objects.files.insert(path.clone(), Vec::new());
        } else if matches!(disposition, 2 | 5) {
            *objects.files.get_mut(&path).expect("existing file") = Vec::new();
        }
        if matches!(disposition, 2 | 4) {
            open.fail(if exists { ALREADY_EXISTS } else { 0 }, 0)?;
        }
        objects.insert_handle(
            handle,
            GuestObject::File(OpenFile {
                path,
                position: 0,
                access,
                share,
            }),
        );
        Ok(handle)
    });

    let size = Arc::clone(state);
    registry.register(KERNEL32, "GetFileSizeEx", move |args, _, memory| {
        let objects = size.objects.lock().expect("guest object state");
        let Some(GuestObject::File(file)) = objects.handles.get(&args.rcx) else {
            return size.fail(ERROR_INVALID_HANDLE, 0);
        };
        let bytes = objects
            .files
            .get(&file.path)
            .expect("open file exists")
            .len();
        if args.rdx == 0 || memory.write_u64(args.rdx, bytes as u64).is_err() {
            return size.fail(ERROR_NOACCESS, 0);
        }
        Ok(1)
    });

    for (symbol, writing) in [("ReadFile", false), ("WriteFile", true)] {
        let state = Arc::clone(state);
        registry.register(KERNEL32, symbol, move |args, _, memory| {
            if args.get(4, memory).map_err(ExitReason::MemoryFault)? != 0 {
                return state.fail(ERROR_NOT_SUPPORTED, 0);
            }
            let count = args.r8 as u32 as usize;
            if count > MAX_TRANSFER {
                return state.fail(ERROR_NOT_SUPPORTED, 0);
            }
            let mut objects = state.objects.lock().expect("guest object state");
            let super::ObjectState { handles, files, .. } = &mut *objects;
            let Some(GuestObject::File(file)) = handles.get_mut(&args.rcx) else {
                return state.fail(ERROR_INVALID_HANDLE, 0);
            };
            let required = if writing { WRITE } else { READ };
            if file.access & required == 0 {
                return state.fail(ERROR_ACCESS_DENIED, 0);
            }
            if args.r9 == 0 || memory.check_write(args.r9, 4).is_err() {
                return state.fail(ERROR_NOACCESS, 0);
            }

            let used = if writing {
                files.values().map(Vec::len).sum::<usize>()
            } else {
                0
            };
            let data = files.get_mut(&file.path).expect("open file exists");
            let position = file.position;
            let transferred = if writing {
                if count == 0 {
                    0
                } else {
                    let end = position + count;
                    let growth = end.saturating_sub(data.len());
                    if growth > MAX_BYTES - used {
                        return state.fail(DISK_FULL, 0);
                    }
                    let mut input = vec![0; count];
                    if args.rdx == 0 || memory.read_bytes(args.rdx, &mut input).is_err() {
                        return state.fail(ERROR_NOACCESS, 0);
                    }
                    if growth != 0 {
                        data.try_reserve_exact(growth)
                            .map_err(|_| ExitReason::MemoryFault(MemoryError::AllocationFailed))?;
                        data.resize(end, 0);
                    }
                    data[position..end].copy_from_slice(&input);
                    count
                }
            } else {
                let start = position.min(data.len());
                let transferred = count.min(data.len() - start);
                if transferred != 0
                    && (args.rdx == 0
                        || memory
                            .write_bytes(args.rdx, &data[start..start + transferred])
                            .is_err())
                {
                    return state.fail(ERROR_NOACCESS, 0);
                }
                transferred
            };
            memory
                .write_u32(args.r9, transferred as u32)
                .map_err(ExitReason::MemoryFault)?;
            file.position += transferred;
            Ok(1)
        });
    }

    for symbol in ["CreateFileW", "GetFileSizeEx", "ReadFile", "WriteFile"] {
        registry.alias(FILE_API, KERNEL32, symbol);
        registry.set_status(KERNEL32, symbol, ServiceStatus::Partial);
    }
}
