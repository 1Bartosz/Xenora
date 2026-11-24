//! Ties the decoder's declared semantic forms to what it decodes from LLVM-assembled fixtures.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use xenora_decoder::{Architecture, SEMANTIC_FORMS, decode};
use xenora_isa::meta::{FORMS, identify};

#[test]
fn declared_semantic_forms_are_exactly_those_decoded_from_fixtures() {
    let directory = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/rdna2");
    let mut decoded = BTreeSet::new();
    for entry in fs::read_dir(directory).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_none_or(|extension| extension != "bin") {
            continue;
        }
        let binary = fs::read(&path).unwrap();
        for instruction in decode(Architecture::Gfx1030, &binary).unwrap() {
            let found = identify(&binary[instruction.offset..]).unwrap();
            assert_eq!(
                found.len,
                usize::from(instruction.byte_len),
                "{}",
                path.display()
            );
            decoded.insert((found.form.name, found.form.family));
        }
    }
    let declared: BTreeSet<_> = SEMANTIC_FORMS.into_iter().collect();
    assert!(
        decoded.is_subset(&declared),
        "{:?}",
        decoded.difference(&declared)
    );
    // Covered by inline words in tests/gfx1030.rs rather than fixture binaries.
    let inline_only: BTreeSet<_> = declared
        .difference(&decoded)
        .map(|(name, _)| *name)
        .collect();
    assert_eq!(
        inline_only,
        BTreeSet::from(["IMAGE_SAMPLE_LZ", "S_CBRANCH_EXECZ", "S_CBRANCH_VCCZ"])
    );
    for (name, family) in SEMANTIC_FORMS {
        assert!(
            FORMS
                .iter()
                .any(|form| form.name == name && form.family == family),
            "{name}"
        );
    }
}
