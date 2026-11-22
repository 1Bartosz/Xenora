// SPDX-License-Identifier: GPL-3.0-or-later

use std::error::Error;

use xenora_metal::NativeCompute;
use xenora_shader::{Architecture, ENTRY_POINT, compile};

const INPUT: [u32; 4] = [0, 1, u32::MAX, 3_989_547_400];

struct Fixture {
    name: &'static str,
    code: &'static [u8],
    expected: [u32; 4],
}

const FIXTURES: [Fixture; 3] = [
    Fixture {
        name: "add",
        code: include_bytes!("../../../fixtures/rdna2/add.bin"),
        expected: [305_419_896, 305_419_897, 305_419_895, 0],
    },
    Fixture {
        name: "bitwise-branch",
        code: include_bytes!("../../../fixtures/rdna2/bitwise-branch.bin"),
        expected: [305_419_896, 305_419_897, 315_315_847, 318_723_824],
    },
    Fixture {
        name: "float-exact",
        code: include_bytes!("../../../fixtures/rdna2/float-exact.bin"),
        expected: [0x410c_0000, 0x410c_0000, 0x4120_0000, 0x4120_0000],
    },
];

pub fn run() -> Result<(), Box<dyn Error>> {
    let metal = NativeCompute::new()?;
    println!("Metal device: {}", metal.device_name());
    for fixture in FIXTURES {
        run_fixture(&metal, fixture)?;
    }
    Ok(())
}

fn run_fixture(metal: &NativeCompute, fixture: Fixture) -> Result<(), Box<dyn Error>> {
    let Fixture {
        name,
        code,
        expected,
    } = fixture;
    println!("Fixture {name}");
    let compiled = compile(Architecture::Gfx1030, code)?;
    println!(
        "Disassembly\n{}\nIR\n{}\nMSL\n{}",
        compiled.disassembly, compiled.ir, compiled.msl
    );

    let reference = xenora_ref::execute(code, &INPUT)?;
    if reference != expected {
        return Err(format!("reference mismatch: {reference:?} != {expected:?}").into());
    }

    let actual = metal.run_words(&compiled.msl, ENTRY_POINT, &INPUT)?;
    if actual != reference || actual != expected {
        return Err(format!("Metal mismatch: {actual:?} != {expected:?}").into());
    }
    println!("Metal, reference and known answers match: {actual:?}");
    Ok(())
}
