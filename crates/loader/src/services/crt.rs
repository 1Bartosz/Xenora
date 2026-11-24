// SPDX-License-Identifier: GPL-3.0-or-later

//! C runtime services that operate on checked guest memory.

use crate::cpu::ExitReason;
use crate::imports::{ImportRegistry, ServiceStatus};
use crate::memory::{GuestMemory, MemoryError, PROCESS_HEAP};

const SCAN_BUDGET: u64 = 1_048_576;
const UCRT: &str = "ucrtbase.dll";
const CRT_STRING_API: &str = "api-ms-win-crt-string-l1-1-0.dll";

#[derive(Clone, Copy)]
enum StringUnit {
    Byte,
    Wide,
}

impl StringUnit {
    fn width(self) -> u64 {
        match self {
            Self::Byte => 1,
            Self::Wide => 2,
        }
    }

    fn read(self, memory: &GuestMemory, base: u64, offset: u64) -> Result<u16, ExitReason> {
        if base == 0 {
            return Err(ExitReason::MemoryFault(MemoryError::UnmappedAddress(0)));
        }
        let address = base
            .checked_add(offset * self.width())
            .ok_or(ExitReason::MemoryFault(MemoryError::Overflow))?;
        match self {
            Self::Byte => memory.read_u8(address).map(u16::from),
            Self::Wide => memory.read_u16(address),
        }
        .map_err(ExitReason::MemoryFault)
    }
}

fn string_length(
    memory: &GuestMemory,
    address: u64,
    limit: u64,
    unit: StringUnit,
) -> Result<u64, ExitReason> {
    let budget = SCAN_BUDGET / unit.width();
    for offset in 0..limit.min(budget) {
        if unit.read(memory, address, offset)? == 0 {
            return Ok(offset);
        }
    }
    if limit <= budget {
        return Ok(limit);
    }
    Err(ExitReason::IllegalInstruction(0xFE))
}

fn compare_strings(
    memory: &GuestMemory,
    left: u64,
    right: u64,
    limit: u64,
    unit: StringUnit,
) -> Result<u64, ExitReason> {
    let budget = SCAN_BUDGET / unit.width();
    for offset in 0..limit.min(budget) {
        let a = unit.read(memory, left, offset)?;
        let b = unit.read(memory, right, offset)?;
        if a != b || a == 0 {
            return Ok(u64::from((i32::from(a) - i32::from(b)) as u32));
        }
    }
    if limit <= budget {
        return Ok(0);
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
    for (length, bounded_length, compare, bounded_compare, unit) in [
        ("strlen", "strnlen", "strcmp", "strncmp", StringUnit::Byte),
        ("wcslen", "wcsnlen", "wcscmp", "wcsncmp", StringUnit::Wide),
    ] {
        registry.register(UCRT, length, move |args, _, memory| {
            string_length(memory, args.rcx, u64::MAX, unit)
        });
        registry.register(UCRT, bounded_length, move |args, _, memory| {
            string_length(memory, args.rcx, args.rdx, unit)
        });
        registry.register(UCRT, compare, move |args, _, memory| {
            compare_strings(memory, args.rcx, args.rdx, u64::MAX, unit)
        });
        registry.register(UCRT, bounded_compare, move |args, _, memory| {
            compare_strings(memory, args.rcx, args.rdx, args.r8, unit)
        });
        for symbol in [length, bounded_length, compare, bounded_compare] {
            registry.alias(CRT_STRING_API, UCRT, symbol);
            registry.set_status(UCRT, symbol, ServiceStatus::Partial);
        }
    }

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

    for symbol in ["memset", "memcpy", "memmove", "memcmp"] {
        registry.alias(CRT_STRING_API, UCRT, symbol);
        registry.set_status(UCRT, symbol, ServiceStatus::Partial);
    }
}

pub(super) fn register_heap(registry: &mut ImportRegistry) {
    registry.register(UCRT, "malloc", |args, _, memory| {
        Ok(memory
            .map_heap_allocation(PROCESS_HEAP, args.rcx.max(1) as usize)
            .unwrap_or(0))
    });
    registry.register(UCRT, "calloc", |args, _, memory| {
        let Some(size) = args.rcx.checked_mul(args.rdx) else {
            return Ok(0);
        };
        Ok(memory
            .map_heap_allocation(PROCESS_HEAP, size.max(1) as usize)
            .unwrap_or(0))
    });
    registry.register(UCRT, "realloc", |args, _, memory| {
        let address = args.rcx;
        let size = args.rdx;
        if address == 0 {
            return Ok(memory
                .map_heap_allocation(PROCESS_HEAP, size.max(1) as usize)
                .unwrap_or(0));
        }
        if size == 0 {
            free(memory, address)?;
            return Ok(0);
        }
        Ok(memory
            .resize_heap_allocation(PROCESS_HEAP, address, size as usize, false)
            .unwrap_or(0))
    });
    registry.register(UCRT, "free", |args, _, memory| {
        free(memory, args.rcx)?;
        Ok(0)
    });

    for symbol in ["malloc", "calloc", "realloc", "free"] {
        registry.alias("api-ms-win-crt-heap-l1-1-0.dll", UCRT, symbol);
        registry.set_status(UCRT, symbol, ServiceStatus::Partial);
    }
}

fn free(memory: &mut GuestMemory, address: u64) -> Result<(), ExitReason> {
    if address == 0 || memory.release_heap_allocation(PROCESS_HEAP, address) {
        Ok(())
    } else {
        Err(ExitReason::MemoryFault(MemoryError::UnmappedAddress(
            address,
        )))
    }
}
