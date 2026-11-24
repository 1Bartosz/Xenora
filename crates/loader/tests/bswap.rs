// SPDX-License-Identifier: GPL-3.0-or-later

use xenora_loader::cpu::{CpuState, ExitReason, Interpreter, RFlags};
use xenora_loader::dbt::DbtEngine;
use xenora_loader::memory::{GuestMemory, MemoryPermissions};

fn check_bswap(native: bool) {
    const ENTRY: u64 = 0x1000;
    const INPUT: u64 = 0x1122_3344_5566_7788;
    for reg in 0_u8..16 {
        for width in [16, 32, 64] {
            let wide = width == 64;
            let mut code = Vec::new();
            if width == 16 {
                code.push(0x66);
            }
            let rex = 0x40 | (u8::from(wide) << 3) | (reg >> 3);
            if rex != 0x40 {
                code.push(rex);
            }
            code.extend([0x0f, 0xc8 + (reg & 7)]);
            let mut memory = GuestMemory::new();
            memory
                .map_region_with_data(ENTRY, code.len(), MemoryPermissions::READ_EXECUTE, &code)
                .unwrap();
            let mut cpu = CpuState::new(ENTRY, 0);
            cpu.gpr.fill(0xdead_beef);
            cpu.gpr[reg as usize] = INPUT;
            cpu.flags = RFlags {
                cf: wide,
                zf: !wide,
                sf: wide,
                of: !wide,
                ..RFlags::default()
            };
            let flags = cpu.flags;
            let mut expected = cpu.gpr;
            let result = if native {
                DbtEngine::new(16 * 1024)
                    .unwrap()
                    .run(&mut cpu, &mut memory, 1)
            } else {
                Interpreter::new(cpu, &mut memory, 1).run()
            };
            if width == 16 {
                assert!(matches!(
                    result.exit_reason,
                    ExitReason::IllegalInstruction(_)
                ));
                assert_eq!(result.instructions_executed, 0);
            } else {
                assert_eq!(
                    result.exit_reason,
                    ExitReason::StepLimitExceeded,
                    "{code:02x?}"
                );
                assert_eq!(result.instructions_executed, 1);
                assert_eq!(result.final_state.rip, ENTRY + code.len() as u64);
                expected[reg as usize] = if wide {
                    0x8877_6655_4433_2211
                } else {
                    0x8877_6655
                };
            }
            assert_eq!(result.final_state.gpr, expected);
            assert_eq!(result.final_state.flags, flags);
        }
    }
}

#[test]
fn interpreter_bswap() {
    check_bswap(false);
}

#[test]
fn native_bswap() {
    check_bswap(true);
}
