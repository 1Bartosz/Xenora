// SPDX-License-Identifier: GPL-3.0-or-later
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|input: &[u8]| {
    let _ = xenora_decoder::decode(xenora_decoder::Architecture::Gfx1030, input);
});
