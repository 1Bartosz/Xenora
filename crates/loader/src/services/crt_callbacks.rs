// SPDX-License-Identifier: GPL-3.0-or-later

//! CRT services that call guest functions: sorting, searching and the
//! initializer tables run during C and C++ static initialization.
//!
//! Each service is a callback task. The comparator or initializer runs as
//! ordinary translated guest code, and the service continues when it returns.

use crate::cpu::ExitReason;
use crate::imports::{CallbackStart, CallbackStep, CallbackTask, ImportRegistry, ServiceStatus};
use crate::memory::{GuestMemory, MemoryError};

const UCRT: &str = "ucrtbase.dll";
const CRT_UTILITY_API: &str = "api-ms-win-crt-utility-l1-1-0.dll";
const CRT_RUNTIME_API: &str = "api-ms-win-crt-runtime-l1-1-0.dll";
/// Largest array, in bytes, that sorting may permute.
const SORT_BUDGET: u64 = 1_048_576;
/// Largest initializer table, in entries.
const MAX_INITIALIZERS: u64 = 65_536;

/// An invalid argument would reach the CRT invalid-parameter handler, which
/// the runtime does not model.
fn unsupported<T>() -> Result<T, ExitReason> {
    Err(ExitReason::IllegalInstruction(0xFE))
}

fn fault(error: MemoryError) -> ExitReason {
    ExitReason::MemoryFault(error)
}

/// The comparator's `int` result.
fn ordering(result: u64) -> i32 {
    result as u32 as i32
}

/// Comparator arguments. The secure variants pass the caller's context first.
#[derive(Clone, Copy)]
struct Comparator {
    entry: u64,
    context: Option<u64>,
}

impl Comparator {
    fn call(self, left: u64, right: u64) -> CallbackStep {
        let args = match self.context {
            Some(context) => [context, left, right, 0],
            None => [left, right, 0, 0],
        };
        CallbackStep::Call {
            entry: self.entry,
            args,
        }
    }
}

/// Binary insertion sort over element indices. Elements stay in place while
/// the comparator runs and are permuted once at the end.
struct Sort {
    base: u64,
    width: u64,
    count: u64,
    compare: Comparator,
    sorted: Vec<u64>,
    next: u64,
    low: usize,
    high: usize,
}

impl Sort {
    fn element(&self, index: u64) -> u64 {
        self.base + index * self.width
    }

    fn probe(&self) -> CallbackStep {
        let middle = (self.low + self.high) / 2;
        self.compare
            .call(self.element(self.next), self.element(self.sorted[middle]))
    }

    fn permute(&self, memory: &mut GuestMemory) -> Result<(), ExitReason> {
        let mut original = vec![0; (self.count * self.width) as usize];
        memory.read_bytes(self.base, &mut original).map_err(fault)?;
        let width = self.width as usize;
        let mut ordered = Vec::with_capacity(original.len());
        for &index in &self.sorted {
            let start = index as usize * width;
            ordered.extend_from_slice(&original[start..start + width]);
        }
        memory.write_bytes(self.base, &ordered).map_err(fault)
    }
}

impl CallbackTask for Sort {
    fn resume(
        &mut self,
        result: u64,
        _: u64,
        memory: &mut GuestMemory,
    ) -> Result<CallbackStep, ExitReason> {
        let middle = (self.low + self.high) / 2;
        // Inserting after equal elements keeps equal keys in input order.
        if ordering(result) < 0 {
            self.high = middle;
        } else {
            self.low = middle + 1;
        }
        if self.low < self.high {
            return Ok(self.probe());
        }
        self.sorted.insert(self.low, self.next);
        self.next += 1;
        if self.next == self.count {
            self.permute(memory)?;
            return Ok(CallbackStep::Return(0));
        }
        self.low = 0;
        self.high = self.sorted.len();
        Ok(self.probe())
    }
}

fn start_sort(
    memory: &mut GuestMemory,
    base: u64,
    count: u64,
    width: u64,
    compare: Comparator,
) -> Result<CallbackStart, ExitReason> {
    if count > 0 && (base == 0 || width == 0) || compare.entry == 0 {
        return unsupported();
    }
    if count < 2 {
        return Ok(CallbackStart::Return(0));
    }
    let Some(bytes) = count
        .checked_mul(width)
        .filter(|&bytes| bytes <= SORT_BUDGET)
    else {
        return unsupported();
    };
    base.checked_add(bytes)
        .ok_or(fault(MemoryError::Overflow))?;
    memory.check_write(base, bytes as usize).map_err(fault)?;
    let sort = Sort {
        base,
        width,
        count,
        compare,
        sorted: vec![0],
        next: 1,
        low: 0,
        high: 1,
    };
    let CallbackStep::Call { entry, args } = sort.probe() else {
        unreachable!("a probe always calls the comparator");
    };
    Ok(CallbackStart::Call {
        task: Box::new(sort),
        entry,
        args,
    })
}

struct Search {
    key: u64,
    base: u64,
    width: u64,
    compare: Comparator,
    low: u64,
    high: u64,
}

impl Search {
    fn middle(&self) -> u64 {
        self.low + (self.high - self.low) / 2
    }

    fn probe(&self) -> CallbackStep {
        self.compare
            .call(self.key, self.base + self.middle() * self.width)
    }
}

impl CallbackTask for Search {
    fn resume(
        &mut self,
        result: u64,
        _: u64,
        _: &mut GuestMemory,
    ) -> Result<CallbackStep, ExitReason> {
        let middle = self.middle();
        match ordering(result) {
            0 => return Ok(CallbackStep::Return(self.base + middle * self.width)),
            order if order < 0 => self.high = middle,
            _ => self.low = middle + 1,
        }
        if self.low < self.high {
            Ok(self.probe())
        } else {
            Ok(CallbackStep::Return(0))
        }
    }
}

fn start_search(
    key: u64,
    base: u64,
    count: u64,
    width: u64,
    compare: Comparator,
) -> Result<CallbackStart, ExitReason> {
    if compare.entry == 0 || count > 0 && (base == 0 || width == 0) {
        return unsupported();
    }
    if count == 0 {
        return Ok(CallbackStart::Return(0));
    }
    count
        .checked_mul(width)
        .and_then(|bytes| base.checked_add(bytes))
        .ok_or(fault(MemoryError::Overflow))?;
    let search = Search {
        key,
        base,
        width,
        compare,
        low: 0,
        high: count,
    };
    let CallbackStep::Call { entry, args } = search.probe() else {
        unreachable!("a probe always calls the comparator");
    };
    Ok(CallbackStart::Call {
        task: Box::new(search),
        entry,
        args,
    })
}

/// Calls each nonnull function pointer in `[cursor, end)` in order.
/// `_initterm_e` stops at the first initializer that returns nonzero.
struct Initializers {
    cursor: u64,
    end: u64,
    stop_on_error: bool,
}

impl Initializers {
    fn next(&mut self, memory: &GuestMemory) -> Result<CallbackStep, ExitReason> {
        while self.cursor < self.end {
            let entry = memory.read_u64(self.cursor).map_err(fault)?;
            self.cursor += 8;
            if entry != 0 {
                return Ok(CallbackStep::Call {
                    entry,
                    args: [0; 4],
                });
            }
        }
        Ok(CallbackStep::Return(0))
    }
}

impl CallbackTask for Initializers {
    fn resume(
        &mut self,
        result: u64,
        _: u64,
        memory: &mut GuestMemory,
    ) -> Result<CallbackStep, ExitReason> {
        let status = result as u32;
        if self.stop_on_error && status != 0 {
            return Ok(CallbackStep::Return(u64::from(status)));
        }
        self.next(memory)
    }
}

fn start_initializers(
    memory: &mut GuestMemory,
    first: u64,
    last: u64,
    stop_on_error: bool,
) -> Result<CallbackStart, ExitReason> {
    let Some(span) = last.checked_sub(first) else {
        return unsupported();
    };
    if !span.is_multiple_of(8) || span / 8 > MAX_INITIALIZERS {
        return unsupported();
    }
    let mut table = Initializers {
        cursor: first,
        end: last,
        stop_on_error,
    };
    Ok(match table.next(memory)? {
        CallbackStep::Call { entry, args } => CallbackStart::Call {
            task: Box::new(table),
            entry,
            args,
        },
        CallbackStep::Return(value) => CallbackStart::Return(value),
    })
}

pub(super) fn register(registry: &mut ImportRegistry) {
    registry.register_callback_task(UCRT, "qsort", ServiceStatus::Partial, |args, _, mem, _| {
        let compare = Comparator {
            entry: args.r9,
            context: None,
        };
        start_sort(mem, args.rcx, args.rdx, args.r8, compare)
    });
    registry.register_callback_task(
        UCRT,
        "qsort_s",
        ServiceStatus::Partial,
        |args, _, mem, _| {
            let context = args.get(4, mem).map_err(fault)?;
            let compare = Comparator {
                entry: args.r9,
                context: Some(context),
            };
            start_sort(mem, args.rcx, args.rdx, args.r8, compare)
        },
    );
    registry.register_callback_task(
        UCRT,
        "bsearch",
        ServiceStatus::Partial,
        |args, _, mem, _| {
            let compare = Comparator {
                entry: args.get(4, mem).map_err(fault)?,
                context: None,
            };
            start_search(args.rcx, args.rdx, args.r8, args.r9, compare)
        },
    );
    registry.register_callback_task(
        UCRT,
        "bsearch_s",
        ServiceStatus::Partial,
        |args, _, mem, _| {
            let compare = Comparator {
                entry: args.get(4, mem).map_err(fault)?,
                context: Some(args.get(5, mem).map_err(fault)?),
            };
            start_search(args.rcx, args.rdx, args.r8, args.r9, compare)
        },
    );
    for (symbol, stop_on_error) in [("_initterm", false), ("_initterm_e", true)] {
        registry.register_callback_task(
            UCRT,
            symbol,
            ServiceStatus::Partial,
            move |args, _, mem, _| start_initializers(mem, args.rcx, args.rdx, stop_on_error),
        );
        registry.alias(CRT_RUNTIME_API, UCRT, symbol);
    }
    for symbol in ["qsort", "qsort_s", "bsearch", "bsearch_s"] {
        registry.alias(CRT_UTILITY_API, UCRT, symbol);
    }
}
