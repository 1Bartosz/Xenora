// SPDX-License-Identifier: GPL-3.0-or-later

//! C runtime services that operate on checked guest memory.

use crate::cpu::ExitReason;
use crate::imports::{ImportRegistry, ServiceStatus};
use crate::memory::{GuestMemory, MemoryError};

const SCAN_BUDGET: u64 = 1_048_576;
const UCRT: &str = "ucrtbase.dll";
const CRT_STRING_API: &str = "api-ms-win-crt-string-l1-1-0.dll";

fn string_length(memory: &GuestMemory, address: u64, limit: u64) -> Result<u64, ExitReason> {
    if limit != 0 && address == 0 {
        return Err(ExitReason::MemoryFault(MemoryError::UnmappedAddress(0)));
    }
    for offset in 0..limit.min(SCAN_BUDGET) {
        let address = address
            .checked_add(offset)
            .ok_or(ExitReason::MemoryFault(MemoryError::Overflow))?;
        if memory.read_u8(address).map_err(ExitReason::MemoryFault)? == 0 {
            return Ok(offset);
        }
    }
    if limit <= SCAN_BUDGET {
        return Ok(limit);
    }
    Err(ExitReason::IllegalInstruction(0xFE))
}

fn transfer_buffer(count: u64, fill: u8) -> Result<Vec<u8>, ExitReason> {
    if count > SCAN_BUDGET {
        return Err(ExitReason::IllegalInstruction(0xFE));
    }
    let count = count as usize;
    let mut buffer = Vec::new();
    buffer
        .try_reserve_exact(count)
        .map_err(|_| ExitReason::MemoryFault(MemoryError::AllocationFailed))?;
    buffer.resize(count, fill);
    Ok(buffer)
}

pub(super) fn register(registry: &mut ImportRegistry) {
    registry.register(UCRT, "strlen", |args, _, memory| {
        string_length(memory, args.rcx, u64::MAX)
    });
    registry.register(UCRT, "strnlen", |args, _, memory| {
        string_length(memory, args.rcx, args.rdx)
    });

    registry.register(UCRT, "memset", |args, _, memory| {
        if args.r8 != 0 {
            let buffer = transfer_buffer(args.r8, args.rdx as u8)?;
            memory
                .write_bytes(args.rcx, &buffer)
                .map_err(ExitReason::MemoryFault)?;
        }
        Ok(args.rcx)
    });

    for symbol in ["memcpy", "memmove"] {
        registry.register(UCRT, symbol, |args, _, memory| {
            if args.r8 != 0 {
                let mut buffer = transfer_buffer(args.r8, 0)?;
                memory
                    .read_bytes(args.rdx, &mut buffer)
                    .map_err(ExitReason::MemoryFault)?;
                memory
                    .write_bytes(args.rcx, &buffer)
                    .map_err(ExitReason::MemoryFault)?;
            }
            Ok(args.rcx)
        });
    }

    registry.register(UCRT, "memcmp", |args, _, memory| {
        if args.r8 == 0 {
            return Ok(0);
        }
        let mut left = transfer_buffer(args.r8, 0)?;
        let mut right = transfer_buffer(args.r8, 0)?;
        memory
            .read_bytes(args.rcx, &mut left)
            .map_err(ExitReason::MemoryFault)?;
        memory
            .read_bytes(args.rdx, &mut right)
            .map_err(ExitReason::MemoryFault)?;
        let result = left
            .iter()
            .zip(right)
            .find_map(|(&a, b)| (a != b).then_some(i32::from(a) - i32::from(b)))
            .unwrap_or(0);
        Ok(u64::from(result as u32))
    });

    for symbol in ["strlen", "strnlen", "memset", "memcpy", "memmove", "memcmp"] {
        registry.alias(CRT_STRING_API, UCRT, symbol);
        registry.set_status(UCRT, symbol, ServiceStatus::Partial);
    }
}
