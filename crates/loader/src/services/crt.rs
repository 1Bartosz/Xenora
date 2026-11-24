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

const ENOMEM: u32 = 12;
const EINVAL: u32 = 22;

fn set_errno(memory: &mut GuestMemory, error: u32) -> Result<(), ExitReason> {
    let address = memory
        .crt_errno_address()
        .map_err(ExitReason::MemoryFault)?;
    memory
        .write_u32(address, error)
        .map_err(ExitReason::MemoryFault)
}

fn allocation_result(
    memory: &mut GuestMemory,
    result: Result<u64, MemoryError>,
) -> Result<u64, ExitReason> {
    match result {
        Ok(address) => Ok(address),
        Err(_) => {
            set_errno(memory, ENOMEM)?;
            Ok(0)
        }
    }
}

const ERANGE: u32 = 34;

fn parse_long(
    memory: &mut GuestMemory,
    address: u64,
    end: u64,
    mut radix: u32,
    signed: bool,
    unit: StringUnit,
) -> Result<u64, ExitReason> {
    if address == 0 || (radix != 0 && !(2..=36).contains(&radix)) {
        if end != 0 {
            memory
                .write_u64(end, address)
                .map_err(ExitReason::MemoryFault)?;
        }
        set_errno(memory, EINVAL)?;
        return Ok(0);
    }

    let read = |index| {
        if index >= SCAN_BUDGET / unit.width() {
            return Err(ExitReason::IllegalInstruction(0xFE));
        }
        unit.read(memory, address, index)
            .map(|value| char::from_u32(u32::from(value)).unwrap_or(char::REPLACEMENT_CHARACTER))
    };
    let mut index = 0;
    let mut next = read(index)?;
    while matches!(next, ' ' | '\t' | '\n' | '\r' | '\x0b' | '\x0c') {
        index += 1;
        next = read(index)?;
    }
    let negative = next == '-';
    if matches!(next, '+' | '-') {
        index += 1;
        next = read(index)?;
    }
    if next == '0'
        && matches!(radix, 0 | 16)
        && matches!(read(index + 1)?, 'x' | 'X')
        && read(index + 2)?.is_ascii_hexdigit()
    {
        radix = 16;
        index += 2;
        next = read(index)?;
    }
    if radix == 0 {
        radix = if next == '0' { 8 } else { 10 };
    }

    let limit = if signed {
        i32::MAX as u32 + u32::from(negative)
    } else {
        u32::MAX
    };
    let start = index;
    let mut value = 0u32;
    let mut overflow = false;
    while let Some(digit) = next.to_digit(radix) {
        match value.checked_mul(radix).and_then(|v| v.checked_add(digit)) {
            Some(v) if v <= limit => value = v,
            _ => overflow = true,
        }
        index += 1;
        next = read(index)?;
    }
    let consumed = if start == index { 0 } else { index };
    if end != 0 {
        let pointer = address
            .checked_add(consumed * unit.width())
            .ok_or(ExitReason::MemoryFault(MemoryError::Overflow))?;
        memory
            .write_u64(end, pointer)
            .map_err(ExitReason::MemoryFault)?;
    }
    if overflow {
        set_errno(memory, ERANGE)?;
        value = limit;
    }
    if negative && (!overflow || signed) {
        value = value.wrapping_neg();
    }
    Ok(u64::from(value))
}

pub(super) fn register_conversions(registry: &mut ImportRegistry) {
    for (symbol, signed, unit) in [
        ("strtol", true, StringUnit::Byte),
        ("strtoul", false, StringUnit::Byte),
        ("wcstol", true, StringUnit::Wide),
        ("wcstoul", false, StringUnit::Wide),
    ] {
        registry.register(UCRT, symbol, move |args, _, memory| {
            parse_long(memory, args.rcx, args.rdx, args.r8 as u32, signed, unit)
        });
        registry.alias("api-ms-win-crt-convert-l1-1-0.dll", UCRT, symbol);
        registry.set_status(UCRT, symbol, ServiceStatus::Partial);
    }
}

pub(super) fn register_errno(registry: &mut ImportRegistry) {
    registry.register(UCRT, "_errno", |_, _, memory| {
        memory.crt_errno_address().map_err(ExitReason::MemoryFault)
    });
    registry.register(UCRT, "_set_errno", |args, _, memory| {
        set_errno(memory, args.rcx as u32)?;
        Ok(0)
    });
    registry.register(UCRT, "_get_errno", |args, _, memory| {
        if args.rcx == 0 {
            set_errno(memory, EINVAL)?;
            return Ok(u64::from(EINVAL));
        }
        let address = memory
            .crt_errno_address()
            .map_err(ExitReason::MemoryFault)?;
        let value = memory.read_u32(address).map_err(ExitReason::MemoryFault)?;
        memory
            .write_u32(args.rcx, value)
            .map_err(ExitReason::MemoryFault)?;
        Ok(0)
    });
    for symbol in ["_errno", "_set_errno", "_get_errno"] {
        registry.alias("api-ms-win-crt-runtime-l1-1-0.dll", UCRT, symbol);
        registry.set_status(UCRT, symbol, ServiceStatus::Partial);
    }
}

pub(super) fn register_heap(registry: &mut ImportRegistry) {
    registry.register(UCRT, "malloc", |args, _, memory| {
        let result = memory.map_heap_allocation(PROCESS_HEAP, args.rcx.max(1) as usize);
        allocation_result(memory, result)
    });
    registry.register(UCRT, "calloc", |args, _, memory| {
        let result = args
            .rcx
            .checked_mul(args.rdx)
            .ok_or(MemoryError::AllocationFailed)
            .and_then(|size| memory.map_heap_allocation(PROCESS_HEAP, size.max(1) as usize));
        allocation_result(memory, result)
    });
    registry.register(UCRT, "realloc", |args, _, memory| {
        let address = args.rcx;
        let size = args.rdx;
        if address == 0 {
            let result = memory.map_heap_allocation(PROCESS_HEAP, size.max(1) as usize);
            return allocation_result(memory, result);
        }
        if size == 0 {
            free(memory, address)?;
            return Ok(0);
        }
        let result = memory.resize_heap_allocation(PROCESS_HEAP, address, size as usize, false);
        allocation_result(memory, result)
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
