// SPDX-License-Identifier: GPL-3.0-or-later

use xenora_shader::{Architecture, CompileError, compile};

const FIXTURE: &[u8] = include_bytes!("../../../fixtures/rdna2/add.bin");

#[test]
fn compiles_original_fixture() {
    let result = compile(Architecture::Gfx1030, FIXTURE).unwrap();
    assert_eq!(result.ir, include_str!("../../../fixtures/golden/add.ir"));
    assert_eq!(
        result.msl,
        include_str!("../../../fixtures/golden/add.metal")
    );
}

#[test]
fn load_wait_is_required_before_using_loaded_data() {
    let mut code = FIXTURE.to_vec();
    code.drain(8..12);
    assert!(matches!(
        compile(Architecture::Gfx1030, &code),
        Err(CompileError::Verify { offset: 16, .. })
    ));
}

#[test]
fn store_wait_is_required_before_termination() {
    let mut code = FIXTURE.to_vec();
    code.drain(36..40);
    assert!(matches!(
        compile(Architecture::Gfx1030, &code),
        Err(CompileError::Verify { offset: 36, .. })
    ));
}

#[test]
fn rejects_unaligned_length_before_decoding() {
    assert!(matches!(
        compile(Architecture::Gfx1030, &FIXTURE[..FIXTURE.len() - 1]),
        Err(CompileError::InvalidLength { .. })
    ));
}

const BRANCH: &[u8] = include_bytes!("../../../fixtures/rdna2/bitwise-branch.bin");
const FLOAT: &[u8] = include_bytes!("../../../fixtures/rdna2/float-exact.bin");

#[test]
fn uniform_branch_selects_each_known_path() {
    let taken = compile(Architecture::Gfx1030, BRANCH).unwrap();
    assert_eq!(
        taken.ir,
        include_str!("../../../fixtures/golden/bitwise-branch.ir")
    );
    assert_eq!(
        taken.msl,
        include_str!("../../../fixtures/golden/bitwise-branch.metal")
    );

    let mut equal = BRANCH.to_vec();
    equal[17] = 0x87; // Compare s8 with 7 instead of 6.
    let fallthrough = compile(Architecture::Gfx1030, &equal).unwrap();
    assert!(fallthrough.ir.contains("branch.uniform 0x0020 taken=false"));
    assert!(!fallthrough.ir.contains("and.b32"));
    assert!(fallthrough.ir.contains("const.u32 0x00000000"));
}

#[test]
fn rejects_branch_into_a_literal_word() {
    let mut invalid = BRANCH.to_vec();
    invalid[20] = 3; // Branch to byte 36, inside the literal move at byte 32.
    assert!(matches!(
        compile(Architecture::Gfx1030, &invalid),
        Err(CompileError::Decode(error)) if error.offset == 20
    ));
}

#[test]
fn compiles_only_proven_exact_float_operations() {
    let result = compile(Architecture::Gfx1030, FLOAT).unwrap();
    assert_eq!(
        result.ir,
        include_str!("../../../fixtures/golden/float-exact.ir")
    );
    assert_eq!(
        result.msl,
        include_str!("../../../fixtures/golden/float-exact.metal")
    );

    let mut invalid = FLOAT.to_vec();
    invalid[16..20].copy_from_slice(&0x007f_ffffu32.to_le_bytes());
    assert!(matches!(
        compile(Architecture::Gfx1030, &invalid),
        Err(CompileError::Verify { offset: 40, .. })
    ));
}
