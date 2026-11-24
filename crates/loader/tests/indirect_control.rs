// SPDX-License-Identifier: GPL-3.0-or-later

use xenora_loader::cpu::{CpuState, ExitReason, Register};
use xenora_loader::dbt::DbtEngine;
use xenora_loader::memory::{GuestMemory, MemoryError, MemoryPermissions};

#[test]
fn native_indirect_transfers_commit_before_target_fetch() {
    const ENTRY: u64 = 0x2000;
    const SLOT: u64 = 0x1800;
    const TARGET: u64 = 0x3000;
    const STACK: u64 = 0x4000;
    const SENTINEL: u64 = 0xabcd_1234;

    // R9 or [RIP - 0x806]. Both memory operands point back to SLOT.
    for (branch, call) in [
        (&[0x41, 0xff, 0xd1][..], true),
        (&[0x41, 0xff, 0xe1][..], false),
        (&[0xff, 0x15, 0xfa, 0xf7, 0xff, 0xff][..], true),
        (&[0xff, 0x25, 0xfa, 0xf7, 0xff, 0xff][..], false),
    ] {
        for target_permissions in [
            Some(MemoryPermissions::READ_EXECUTE),
            Some(MemoryPermissions::READ_WRITE),
            None,
        ] {
            let return_address = ENTRY + branch.len() as u64;
            let mut code = branch.to_vec();
            code.push(0xf4);
            let mut memory = GuestMemory::new();
            memory
                .map_region_with_data(ENTRY, code.len(), MemoryPermissions::READ_EXECUTE, &code)
                .unwrap();
            memory
                .map_region_with_data(
                    SLOT,
                    8,
                    MemoryPermissions::READ_WRITE,
                    &TARGET.to_le_bytes(),
                )
                .unwrap();
            memory
                .map_region_with_data(
                    STACK,
                    8,
                    MemoryPermissions::READ_WRITE,
                    &SENTINEL.to_le_bytes(),
                )
                .unwrap();
            if let Some(permissions) = target_permissions {
                let target_opcode = if call { 0xc3 } else { 0xf4 };
                memory
                    .map_region_with_data(TARGET, 1, permissions, &[target_opcode])
                    .unwrap();
            }
            let mut cpu = CpuState::new(ENTRY, STACK + 8);
            cpu.set(Register::R9, TARGET);
            let mut engine = DbtEngine::new(16 * 1024).unwrap();
            let transfer = engine.run(&mut cpu, &mut memory, 1);
            assert_eq!(transfer.exit_reason, ExitReason::StepLimitExceeded);
            assert_eq!(transfer.instructions_executed, 1);
            assert_eq!(cpu.rip, TARGET);
            let stack_after_transfer = if call { STACK } else { STACK + 8 };
            assert_eq!(cpu.get(Register::Rsp), stack_after_transfer);
            assert_eq!(
                memory.read_u64(STACK).unwrap(),
                if call { return_address } else { SENTINEL }
            );

            let resumed = engine.run(&mut cpu, &mut memory, 3);
            match target_permissions {
                Some(MemoryPermissions::READ_EXECUTE) => {
                    assert_eq!(resumed.exit_reason, ExitReason::Halt);
                    assert_eq!(resumed.instructions_executed, if call { 2 } else { 1 });
                    assert_eq!(cpu.rip, if call { return_address + 1 } else { TARGET + 1 });
                    assert_eq!(cpu.get(Register::Rsp), STACK + 8);
                }
                permissions => {
                    let fault = if permissions.is_some() {
                        MemoryError::PermissionViolation {
                            address: TARGET,
                            required: "execute",
                        }
                    } else {
                        MemoryError::UnmappedAddress(TARGET)
                    };
                    assert_eq!(resumed.exit_reason, ExitReason::MemoryFault(fault));
                    assert_eq!(resumed.instructions_executed, 0);
                    assert_eq!(cpu.rip, TARGET);
                    assert_eq!(cpu.get(Register::Rsp), stack_after_transfer);
                }
            }
        }
    }
}
