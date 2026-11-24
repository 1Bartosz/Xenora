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

pub(super) fn register(registry: &mut ImportRegistry) {
    registry.register(UCRT, "strlen", |args, _, memory| {
        string_length(memory, args.rcx, u64::MAX)
    });
    registry.register(UCRT, "strnlen", |args, _, memory| {
        string_length(memory, args.rcx, args.rdx)
    });

    for symbol in ["strlen", "strnlen"] {
        registry.alias(CRT_STRING_API, UCRT, symbol);
        registry.set_status(UCRT, symbol, ServiceStatus::Partial);
    }
}
