// SPDX-License-Identifier: GPL-3.0-or-later

use std::sync::Arc;
use xenora_loader::cpu::{CpuState, ExitReason};
use xenora_loader::imports::{ImportRegistry, ServiceStatus, Win64Args};
use xenora_loader::memory::{GuestMemory, MemoryPermissions};
use xenora_loader::services::{MockClock, ServicesState, register_standard_services};

#[test]
fn test_generated_crt_math_in_runtime_registry() {
    let mut registry = ImportRegistry::new();
    let state = Arc::new(ServicesState::new(Arc::new(MockClock::new(100))));
    register_standard_services(&mut registry, state);

    // Register a sample generated CRT handler (e.g. sqrt)
    registry.register_with_status(
        "api-ms-win-crt-math-l1-1-0.dll",
        "sqrt",
        ServiceStatus::Partial,
        |args: &Win64Args,
         _cpu: &mut CpuState,
         _mem: &mut GuestMemory|
         -> Result<u64, ExitReason> {
            let x = f64::from_bits(args.rcx);
            let r = x.sqrt();
            Ok(r.to_bits())
        },
    );

    let mut cpu = CpuState::new(0x1000, 0x8000);
    let mut mem = GuestMemory::new();
    mem.map_region(0x1000, 0x10000, MemoryPermissions::READ_WRITE_EXECUTE)
        .unwrap();

    let args = Win64Args {
        rcx: (16.0f64).to_bits(),
        rdx: 0,
        r8: 0,
        r9: 0,
        rsp: 0x8000,
    };

    let desc = registry
        .binding("api-ms-win-crt-math-l1-1-0.dll", "sqrt")
        .unwrap();
    let res = (desc.handler)(&args, &mut cpu, &mut mem).unwrap();
    assert_eq!(f64::from_bits(res), 4.0);
}

#[test]
fn test_unsupported_stub_fails_explicitly() {
    let mut registry = ImportRegistry::new();
    registry.register_with_status(
        "kernel32.dll",
        "UnsupportedApi",
        ServiceStatus::Stub,
        |_args: &Win64Args,
         _cpu: &mut CpuState,
         _mem: &mut GuestMemory|
         -> Result<u64, ExitReason> { Err(ExitReason::IllegalInstruction(0xFE)) },
    );

    let mut cpu = CpuState::new(0x1000, 0x8000);
    let mut mem = GuestMemory::new();
    mem.map_region(0x1000, 0x10000, MemoryPermissions::READ_WRITE_EXECUTE)
        .unwrap();

    let args = Win64Args {
        rcx: 0,
        rdx: 0,
        r8: 0,
        r9: 0,
        rsp: 0x8000,
    };
    let desc = registry.binding("kernel32.dll", "UnsupportedApi").unwrap();
    let res = (desc.handler)(&args, &mut cpu, &mut mem);
    assert_eq!(res, Err(ExitReason::IllegalInstruction(0xFE)));
}
