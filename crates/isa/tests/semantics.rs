//! Ties the generated semantic registry tables to the independent XML metadata table.

use xenora_isa::ScalarAluOp;
use xenora_isa::meta::{EncodingFamily, FORMS as META_FORMS};
use xenora_isa::semantics::{CarryIn, FORM_IDS, FORMS, GENERATED_SCHEMA, SCHEMA};

#[test]
fn generated_tables_match_the_schema_and_the_metadata_forms() {
    assert_eq!(GENERATED_SCHEMA, SCHEMA);
    assert_eq!(FORM_IDS.len(), FORMS.len());
    for (row, (name, family)) in FORMS.iter().zip(FORM_IDS) {
        assert_eq!((row.name, row.family), (name, family));
        let meta = META_FORMS
            .iter()
            .find(|form| form.name == row.name && form.family == row.family)
            .unwrap_or_else(|| panic!("{} is not an identified form", row.name));
        assert_eq!(meta.opcode, row.opcode, "{}", row.name);
    }
}

#[test]
fn scalar_alu_dispatch_selects_exactly_the_registry_sop2_forms() {
    assert_eq!(ScalarAluOp::ALL.len(), FORMS.len());
    for op in ScalarAluOp::ALL {
        let row = op.semantics();
        assert_eq!(row.name.to_ascii_lowercase(), op.mnemonic());
        assert_eq!(op.reads_scc(), row.carry_in == CarryIn::Scc);
    }
    let mut selected = 0;
    for meta in META_FORMS
        .iter()
        .filter(|form| form.family == EncodingFamily::Sop2)
    {
        let word = meta.bits as u32;
        match ScalarAluOp::from_word(word) {
            Some(op) => {
                assert_eq!(op.semantics().name, meta.name);
                selected += 1;
            }
            None => assert!(
                FORMS.iter().all(|row| row.name != meta.name),
                "{} is declared but not dispatched",
                meta.name
            ),
        }
    }
    assert_eq!(selected, FORMS.len());
    // Other encodings never reach the SOP2 dispatch.
    for meta in META_FORMS
        .iter()
        .filter(|form| form.family != EncodingFamily::Sop2 && form.bytes == 4)
    {
        assert_eq!(
            ScalarAluOp::from_word(meta.bits as u32),
            None,
            "{}",
            meta.name
        );
    }
}
