// SPDX-License-Identifier: GPL-3.0-or-later
use crate::cpu::State;

pub(crate) const WRAP: &[u8] = include_bytes!("../fixtures/wrap.bin");
pub(crate) const NONWRAP: &[u8] = include_bytes!("../fixtures/nonwrap.bin");
pub(crate) const CODE_BASE: u64 = 0x1000;
pub(crate) const OUTPUT_BASE: u64 = 0x2000;
pub(crate) const OUTPUT: [u8; 8] = [0xa5; 8];

pub(crate) fn initial() -> State {
    State {
        // Architectural encoding order, from RAX through R15.
        gpr: [
            0xaabb_ccdd_1234_5678,
            1,
            2,
            3,
            4,
            5,
            6,
            OUTPUT_BASE,
            8,
            9,
            10,
            11,
            12,
            13,
            14,
            15,
        ],
        rip: CODE_BASE,
        rflags: 2,
    }
}
