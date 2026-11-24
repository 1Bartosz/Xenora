// SPDX-License-Identifier: GPL-3.0-or-later

//! Winsock lifecycle for the synthetic guest. Descriptors are guest-owned.

use super::ServicesState;
use crate::imports::{ImportRegistry, ServiceStatus};
use std::collections::BTreeSet;
use std::sync::Arc;

const MODULE: &str = "ws2_32.dll";
const VERSION: u16 = 0x0202;
const WSADATA_SIZE: usize = 408;
const MAX_SOCKETS: usize = 64;
const SOCKET_ERROR: u64 = u32::MAX as u64;
const INVALID_SOCKET: u64 = u64::MAX;
const WSAEFAULT: u32 = 10014;
const WSAEMFILE: u32 = 10024;
const WSAENOTSOCK: u32 = 10038;
const WSAEPROTONOSUPPORT: u32 = 10043;
const WSAESOCKTNOSUPPORT: u32 = 10044;
const WSAEAFNOSUPPORT: u32 = 10047;
const WSASYSNOTREADY: u32 = 10091;
const WSAVERNOTSUPPORTED: u32 = 10092;
const WSANOTINITIALISED: u32 = 10093;

#[derive(Default)]
pub(super) struct Winsock {
    refs: u32,
    serial: u32,
    sockets: BTreeSet<u64>,
}

pub(super) fn register(registry: &mut ImportRegistry, state: &Arc<ServicesState>) {
    let startup = Arc::clone(state);
    registry.register(MODULE, "WSAStartup", move |args, _, memory| {
        let requested = args.rcx as u16;
        let negotiated = match requested.to_le_bytes() {
            [2, minor] => u16::from_le_bytes([2, minor.min(2)]),
            _ => return Ok(u64::from(WSAVERNOTSUPPORTED)),
        };
        let mut net = startup.winsock.lock().expect("guest Winsock state");
        let Some(next) = net.refs.checked_add(1) else {
            return Ok(u64::from(WSASYSNOTREADY));
        };
        let mut data = [0; WSADATA_SIZE];
        data[..2].copy_from_slice(&negotiated.to_le_bytes());
        data[2..4].copy_from_slice(&VERSION.to_le_bytes());
        let description = b"Xenora synthetic Winsock";
        data[16..16 + description.len()].copy_from_slice(description);
        if args.rdx == 0 || memory.write_bytes(args.rdx, &data).is_err() {
            return Ok(u64::from(WSAEFAULT));
        }
        net.refs = next;
        Ok(0)
    });

    let cleanup = Arc::clone(state);
    registry.register(MODULE, "WSACleanup", move |_, cpu, _| {
        let mut net = cleanup.winsock.lock().expect("guest Winsock state");
        if net.refs == 0 {
            return cleanup.fail(cpu.gs_base, WSANOTINITIALISED, SOCKET_ERROR);
        }
        net.refs -= 1;
        if net.refs == 0 {
            net.sockets.clear();
        }
        Ok(0)
    });

    let open = Arc::clone(state);
    registry.register(MODULE, "socket", move |args, cpu, _| {
        let mut net = open.winsock.lock().expect("guest Winsock state");
        let fail = |error| open.fail(cpu.gs_base, error, INVALID_SOCKET);
        if net.refs == 0 {
            return fail(WSANOTINITIALISED);
        }
        if args.rcx as u32 != 2 {
            return fail(WSAEAFNOSUPPORT);
        }
        if args.rdx as u32 != 2 {
            return fail(WSAESOCKTNOSUPPORT);
        }
        if !matches!(args.r8 as u32, 0 | 17) {
            return fail(WSAEPROTONOSUPPORT);
        }
        if net.sockets.len() >= MAX_SOCKETS {
            return fail(WSAEMFILE);
        }
        let Some(next) = net.serial.checked_add(1) else {
            return fail(WSAEMFILE);
        };
        net.serial = next;
        let socket = 0x5300_0000_0000_0000 | u64::from(next);
        net.sockets.insert(socket);
        Ok(socket)
    });

    let close = Arc::clone(state);
    registry.register(MODULE, "closesocket", move |args, cpu, _| {
        let mut net = close.winsock.lock().expect("guest Winsock state");
        if net.refs == 0 {
            return close.fail(cpu.gs_base, WSANOTINITIALISED, SOCKET_ERROR);
        }
        if !net.sockets.remove(&args.rcx) {
            return close.fail(cpu.gs_base, WSAENOTSOCK, SOCKET_ERROR);
        }
        Ok(0)
    });

    let get = Arc::clone(state);
    registry.register(MODULE, "WSAGetLastError", move |_, cpu, _| {
        Ok(u64::from(get.last_error(cpu.gs_base)))
    });
    let set = Arc::clone(state);
    registry.register(MODULE, "WSASetLastError", move |args, cpu, _| {
        set.set_last_error(cpu.gs_base, args.rcx as u32);
        Ok(0)
    });
    for symbol in ["WSAStartup", "WSACleanup", "socket", "closesocket"] {
        registry.set_status(MODULE, symbol, ServiceStatus::Partial);
    }
}
