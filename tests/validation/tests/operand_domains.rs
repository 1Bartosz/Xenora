// SPDX-License-Identifier: GPL-3.0-or-later

//! Operand-domain verification for the supported SOP2 and VOPC forms (part of
//! #215, stage E).
//!
//! Every case runs three ways: the expected value written out below from the
//! public AMD "RDNA 2" Instruction Set Architecture Reference Guide, the
//! reference interpreter `xenora_ref`, and the translated MSL on native Metal.
//! The expected values come only from `oracle`, which imports nothing from the
//! ISA crate, the decoder, the shader compiler or the semantic registry, so a
//! mistake there cannot also produce the expectation.

#[path = "operand_domains/harness.rs"]
mod harness;
#[path = "operand_domains/oracle.rs"]
mod oracle;
#[path = "operand_domains/sop2.rs"]
mod sop2;
#[path = "operand_domains/vopc.rs"]
mod vopc;

use harness::{Case, Rejection};

fn cases() -> Vec<Case> {
    let mut all = sop2::cases();
    all.extend(vopc::cases());
    all
}

fn rejections() -> Vec<Rejection> {
    let mut all = sop2::rejections();
    all.extend(vopc::rejections());
    all
}

#[test]
fn operand_domains_match_the_guide_on_the_reference() {
    let cases = cases();
    harness::assert_counts(&cases, sop2::COUNTS.iter().chain(vopc::COUNTS));
    harness::check_reference(&cases).assert_none("reference disagrees with the guide");
    eprintln!("operand_domains: {} reference cases", cases.len());
    harness::print_counts(&cases);
}

#[test]
fn unsupported_and_truncated_operands_are_rejected() {
    let rejections = rejections();
    assert_eq!(rejections.len(), sop2::REJECTIONS + vopc::REJECTIONS);
    harness::check_rejected(&rejections).assert_none("not rejected as the guide requires");
    eprintln!("operand_domains: {} rejections", rejections.len());
}

#[test]
#[ignore = "requires a native Metal device with API and shader validation"]
fn operand_domains_match_the_guide_on_native_metal() {
    assert_eq!(std::env::var("MTL_DEBUG_LAYER").as_deref(), Ok("1"));
    assert_eq!(std::env::var("MTL_SHADER_VALIDATION").as_deref(), Ok("1"));
    let cases = cases();
    let (failures, programs) = harness::check_metal(&cases);
    failures.assert_none("native Metal disagrees with the guide");
    eprintln!(
        "operand_domains: {} native Metal cases ({programs} distinct programs)",
        cases.len()
    );
}
