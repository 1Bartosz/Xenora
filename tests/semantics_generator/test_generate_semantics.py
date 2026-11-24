# SPDX-License-Identifier: GPL-3.0-or-later
"""Unit tests for crates/isa/generate_semantics.py.

Each rejection rule is fed a small in-memory registry that breaks exactly that
rule, checked against a tiny hand-written XML snapshot. The real registry and
snapshot are only read, never edited. Run from the repository root:

    python3 -m unittest discover -s tests/semantics_generator
"""

import copy
import importlib.util
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "crates/isa/generate_semantics.py"

_spec = importlib.util.spec_from_file_location("generate_semantics", SCRIPT)
gen = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(gen)

SHA = "0" * 64

# Minimal stand-in for crates/isa/data/rdna2.jsonl: the SOP2 encoding and the
# default variants of the four forms the fixture declares (values copied from
# the AMD XML snapshot).
U32 = "FMT_NUM_U32"
I32 = "FMT_NUM_I32"


def _operands(data_format, scc_in=False, src1_format=None):
    rows = [
        ["SDST", data_format, "OPR_SDST", 32, False, True, False, True],
        ["SSRC0", data_format, "OPR_SSRC_NOLDS", 32, True, False, False, True],
        ["SSRC1", src1_format or data_format, "OPR_SSRC_NOLDS", 32, True, False, False, True],
        ["", "FMT_NUM_B1", "OPR_SSRC_SPECIAL_SCC", 1, False, True, True, True],
    ]
    if scc_in:
        rows.append(["", "FMT_NUM_B1", "OPR_SSRC_SPECIAL_SCC", 1, True, False, True, True])
    return rows


def snapshot():
    sop2_base = 0x80000000
    encoding = {
        "name": "ENC_SOP2",
        "bits": 32,
        "fields": [
            ["SSRC0", [[0, 8]]], ["SSRC1", [[8, 8]]], ["SDST", [[16, 7]]],
            ["OP", [[23, 7]]], ["ENCODING", [[30, 2]]],
        ],
        "identifier_mask": 0xFF800000,
        "identifiers": [sop2_base | (op << 23) for op in (0, 2, 4, 14, 34)],
    }
    variants = {}
    for name, opcode, operands in (
        ("S_ADD_U32", 0, _operands(U32)),
        ("S_ADD_I32", 2, _operands(I32)),
        ("S_ADDC_U32", 4, _operands(U32, scc_in=True)),
        ("S_AND_B32", 14, _operands(U32)),
        # The XML gives S_ASHR_I32 a signed S0 and an unsigned shift count.
        ("S_ASHR_I32", 34, _operands(I32, src1_format=U32)),
    ):
        variants[(name, "ENC_SOP2")] = {
            "instruction": name, "encoding": "ENC_SOP2", "opcode": opcode, "operands": operands,
        }
    sopc = {
        "name": "ENC_SOPC",
        "bits": 32,
        "fields": [
            ["SSRC0", [[0, 8]]], ["SSRC1", [[8, 8]]], ["OP", [[16, 7]]], ["ENCODING", [[23, 9]]],
        ],
        "identifier_mask": 0xFFFF0000,
        "identifiers": [0xBF000000 | (op << 16) for op in (4, 10)],
    }
    for name, opcode, data_format in (("S_CMP_LT_I32", 4, I32), ("S_CMP_LT_U32", 10, U32)):
        variants[(name, "ENC_SOPC")] = {
            "instruction": name, "encoding": "ENC_SOPC", "opcode": opcode,
            "operands": _operands(data_format)[1:],
        }
    sop1 = {
        "name": "ENC_SOP1",
        "bits": 32,
        "fields": [
            ["SSRC0", [[0, 8]]], ["OP", [[8, 8]]], ["SDST", [[16, 7]]], ["ENCODING", [[23, 9]]],
        ],
        "identifier_mask": 0xFF80FF00,
        "identifiers": [0xBE800000 | (op << 8) for op in (7, 11, 25)],
    }

    def unary(dst, src, size, scc):
        rows = [
            ["SDST", dst, "OPR_SDST", 32, False, True, False, True],
            ["SSRC0", src, "OPR_SSRC_NOLDS", size, True, False, False, True],
        ]
        if scc:
            rows.append(["", "FMT_NUM_B1", "OPR_SSRC_SPECIAL_SCC", 1, False, True, True, True])
        return rows

    for name, opcode, operands in (
        ("S_NOT_B32", 7, unary(U32, U32, 32, True)),
        ("S_BREV_B32", 11, unary(U32, U32, 32, False)),
        ("S_SEXT_I32_I8", 25, unary(I32, "FMT_NUM_I8", 8, False)),
    ):
        variants[(name, "ENC_SOP1")] = {
            "instruction": name, "encoding": "ENC_SOP1", "opcode": opcode, "operands": operands,
        }
    return {
        "source": {"sha256": SHA},
        "encodings": {"ENC_SOP2": encoding, "ENC_SOPC": sopc, "ENC_SOP1": sop1},
        "variants": variants,
    }


def _form(name, opcode, variant, operation, signedness, scc, carry_in="none"):
    return {
        "name": name, "family": "SOP2", "opcode": opcode, "variant": variant,
        "operation": operation, "signedness": signedness, "carry_in": carry_in,
        "width": 32, "scc": scc, "vcc": "unchanged", "exec": "unchanged", "wave": "any",
        "operands": "sop2_sgpr",
        "spec": [{"document": "guide", "section": "12.1", "page": 101}],
        "verify": ["reference", "independent"],
    }


def unaries():
    reg = registry()
    rule = copy.deepcopy(gen.OPERAND_RULES["sop1_sgpr"]["shape"])
    rule["unsupported"] = sorted(gen.OPERAND_RULES["sop1_sgpr"]["unsupported"])
    reg["operand_rules"]["sop1_sgpr"] = rule

    def unary(name, opcode, variant, operation, signedness, scc, **extra):
        form = _form(name, opcode, variant, operation, signedness, scc)
        form.update(family="SOP1", operands="sop1_sgpr", **extra)
        return form

    reg["form"] += [
        unary("S_NOT_B32", 7, "NotB32", "not", "bitwise", "nonzero"),
        unary("S_BREV_B32", 11, "BrevB32", "brev", "bitwise", "unchanged"),
        unary("S_SEXT_I32_I8", 25, "SextI32I8", "sext", "signed", "unchanged", source_bits=8),
    ]
    return reg


def compares():
    reg = registry()
    reg["form"] += [
        _compare("S_CMP_LT_I32", 4, "CmpLtI32", "lt", "signed"),
        _compare("S_CMP_LT_U32", 10, "CmpLtU32", "lt", "unsigned"),
    ]
    return reg


def _compare(name, opcode, variant, relation, signedness):
    form = _form(name, opcode, variant, "compare", signedness, "relation")
    form.update(family="SOPC", relation=relation, operands="sopc_scalar")
    return form


def registry():
    rule = copy.deepcopy(gen.OPERAND_RULES["sop2_sgpr"]["shape"])
    rule["unsupported"] = sorted(gen.UNSUPPORTED_COMBOS)
    compare_rule = copy.deepcopy(gen.OPERAND_RULES["sopc_scalar"]["shape"])
    compare_rule["unsupported"] = sorted(gen.OPERAND_RULES["sopc_scalar"]["unsupported"])
    return {
        "schema": 1,
        "architecture": "gfx1030",
        "source_sha256": SHA,
        "documents": copy.deepcopy(gen.PINNED_DOCUMENTS),
        "operand_rules": {"sop2_sgpr": rule, "sopc_scalar": compare_rule},
        "form": [
            _form("S_ADD_U32", 0, "AddU32", "add", "unsigned", "carry_out"),
            _form("S_ADD_I32", 2, "AddI32", "add", "signed", "signed_overflow"),
            _form("S_ADDC_U32", 4, "AddcU32", "add", "unsigned", "carry_out", carry_in="scc"),
            _form("S_AND_B32", 14, "AndB32", "and", "bitwise", "nonzero"),
            dict(
                _form("S_ASHR_I32", 34, "AshrI32", "ashr", "signed", "nonzero"),
                shift_count_bits=5,
            ),
        ],
    }


def form(reg, name):
    return next(item for item in reg["form"] if item["name"] == name)


class Validate(unittest.TestCase):
    def errors(self, reg, snap=None):
        return gen.validate(reg, snap if snap is not None else snapshot())

    def rejects(self, reg, fragment, snap=None):
        errors = self.errors(reg, snap)
        self.assertTrue(
            any(fragment in error for error in errors),
            f"expected an error containing {fragment!r}, got {errors}",
        )

    # ---- acceptance ----

    def test_accepts_the_fixture(self):
        self.assertEqual(self.errors(registry()), [])

    def test_accepts_the_real_registry(self):
        self.assertEqual(gen.validate(gen.load_registry(), gen.load_snapshot()), [])

    # ---- top level ----

    def test_rejects_wrong_schema(self):
        reg = registry()
        reg["schema"] = 2
        self.rejects(reg, "schema 2 is not 1")

    def test_rejects_source_sha256_mismatch(self):
        reg = registry()
        reg["source_sha256"] = "f" * 64
        self.rejects(reg, "source_sha256 does not match")

    def test_rejects_unknown_top_level_key(self):
        reg = registry()
        reg["notes"] = "x"
        self.rejects(reg, "unknown top-level keys ['notes']")

    def test_rejects_empty_registry(self):
        reg = registry()
        reg["form"] = []
        self.rejects(reg, "declares no forms")

    # ---- incomplete or unknown fields ----

    def test_rejects_missing_required_field(self):
        reg = registry()
        del form(reg, "S_ADD_U32")["scc"]
        self.rejects(reg, "S_ADD_U32: incomplete, missing ['scc']")

    def test_rejects_unknown_form_key(self):
        reg = registry()
        form(reg, "S_ADD_U32")["expression"] = "a + b"
        self.rejects(reg, "S_ADD_U32: unknown keys ['expression']")

    # ---- closed vocabularies ----

    def test_rejects_unknown_operation(self):
        reg = registry()
        form(reg, "S_ADD_U32")["operation"] = "mul"
        self.rejects(reg, "S_ADD_U32: unknown operation 'mul'")

    def test_rejects_unknown_scc_effect(self):
        reg = registry()
        form(reg, "S_ADD_U32")["scc"] = "overflow"
        self.rejects(reg, "S_ADD_U32: unknown scc 'overflow'")

    def test_rejects_unknown_mask_effect(self):
        reg = registry()
        form(reg, "S_ADD_U32")["vcc"] = "written"
        self.rejects(reg, "S_ADD_U32: unknown vcc 'written'")

    def test_rejects_unsupported_width(self):
        reg = registry()
        form(reg, "S_ADD_U32")["width"] = 64
        self.rejects(reg, "S_ADD_U32: width 64 is not supported")

    def test_rejects_unknown_evidence(self):
        reg = registry()
        form(reg, "S_ADD_U32")["verify"] = ["reference", "hardware"]
        self.rejects(reg, "S_ADD_U32: unknown verification requirement")

    def test_rejects_empty_evidence(self):
        reg = registry()
        form(reg, "S_ADD_U32")["verify"] = []
        self.rejects(reg, "S_ADD_U32: no verification requirement")

    # ---- contradictions ----

    def test_rejects_bitwise_operation_declared_signed(self):
        reg = registry()
        form(reg, "S_AND_B32")["signedness"] = "signed"
        self.rejects(reg, "S_AND_B32: signedness signed contradicts operation and")

    def test_rejects_carry_in_on_logical_operation(self):
        reg = registry()
        form(reg, "S_AND_B32")["carry_in"] = "scc"
        self.rejects(reg, "S_AND_B32: carry_in scc contradicts and/bitwise")

    def test_rejects_carry_out_on_signed_form(self):
        reg = registry()
        form(reg, "S_ADD_I32")["scc"] = "carry_out"
        self.rejects(reg, "S_ADD_I32: scc carry_out contradicts add/signed")

    def test_rejects_scc_unchanged_on_a_form_that_writes_it(self):
        # Declared as preserving SCC while the primitive and the XML both write it.
        reg = registry()
        form(reg, "S_ADD_U32")["scc"] = "unchanged"
        self.rejects(reg, "S_ADD_U32: scc unchanged contradicts add/unsigned")
        self.rejects(reg, "S_ADD_U32: scc unchanged contradicts the XML SCC output")

    def test_rejects_carry_in_the_xml_does_not_read(self):
        reg = registry()
        form(reg, "S_ADD_U32")["carry_in"] = "scc"
        self.rejects(reg, "S_ADD_U32: carry_in scc contradicts the XML SCC input")

    def test_rejects_signedness_contradicting_xml_format(self):
        reg = registry()
        f = form(reg, "S_ADD_I32")
        f["signedness"], f["scc"] = "unsigned", "carry_out"
        self.rejects(reg, "S_ADD_I32: signedness unsigned contradicts XML format FMT_NUM_I32")

    # ---- identity against the XML snapshot ----

    def test_rejects_opcode_mismatch(self):
        reg = registry()
        form(reg, "S_ADD_U32")["opcode"] = 1
        self.rejects(reg, "S_ADD_U32: opcode 1 disagrees with the XML opcode 0")

    def test_rejects_family_mismatch(self):
        reg = registry()
        form(reg, "S_ADD_U32")["family"] = "SOPC"
        self.rejects(reg, "S_ADD_U32: operand rule sop2_sgpr is for SOP2")
        self.rejects(reg, "S_ADD_U32: no default ENC_SOPC variant in the XML snapshot")
        form(reg, "S_ADD_U32")["family"] = "SOPK"
        self.rejects(reg, "S_ADD_U32: family SOPK has no XML encoding ENC_SOPK")

    def test_rejects_form_absent_from_the_xml(self):
        reg = registry()
        reg["form"].append(_form("S_MUL_I32", 38, "MulI32", "add", "signed", "signed_overflow"))
        self.rejects(reg, "S_MUL_I32: no default ENC_SOP2 variant in the XML snapshot")

    # ---- duplicates ----

    def test_rejects_duplicate_name(self):
        reg = registry()
        reg["form"].append(copy.deepcopy(form(reg, "S_ADD_U32")))
        self.rejects(reg, "S_ADD_U32: duplicate name S_ADD_U32")

    def test_rejects_same_encoding_claimed_twice(self):
        reg = registry()
        twin = copy.deepcopy(form(reg, "S_ADD_U32"))
        twin["name"], twin["variant"] = "S_ADD_U32_COPY", "AddU32Copy"
        reg["form"].append(twin)
        self.rejects(reg, "S_ADD_U32_COPY: duplicate opcode ('SOP2', 0)")

    def test_rejects_duplicate_variant(self):
        reg = registry()
        form(reg, "S_ADD_I32")["variant"] = "AddU32"
        self.rejects(reg, "S_ADD_I32: duplicate variant AddU32")

    # ---- references ----

    def test_rejects_missing_guide_reference(self):
        reg = registry()
        form(reg, "S_ADD_U32")["spec"] = [{"document": "xml", "section": "1", "page": 0}]
        self.rejects(reg, "S_ADD_U32: missing a guide section and page reference")

    def test_rejects_undeclared_document(self):
        reg = registry()
        form(reg, "S_ADD_U32")["spec"].append({"document": "blog", "section": "1", "page": 1})
        self.rejects(reg, "S_ADD_U32: spec cites undeclared document 'blog'")

    # ---- operand rules ----

    def test_rejects_unknown_operand_rule_on_a_form(self):
        reg = registry()
        form(reg, "S_ADD_U32")["operands"] = "vop2_vgpr"
        self.rejects(reg, "S_ADD_U32: unknown operand rule 'vop2_vgpr'")

    def test_rejects_operand_rule_the_decoder_does_not_implement(self):
        reg = registry()
        reg["operand_rules"]["vop2_vgpr"] = {"family": "VOP2"}
        self.rejects(reg, "operand rule vop2_vgpr: not implemented by the decoder")

    def test_rejects_operand_rule_shape_differing_from_the_decoder(self):
        reg = registry()
        reg["operand_rules"]["sop2_sgpr"]["max_literals"] = 2
        self.rejects(reg, "operand rule sop2_sgpr: declaration differs")

    def test_rejects_a_shift_without_a_count_width(self):
        reg = registry()
        del reg["form"][-1]["shift_count_bits"]
        self.rejects(reg, "S_ASHR_I32: shift_count_bits must be log2 of the width (32), got None")

    def test_rejects_a_wrong_count_width(self):
        reg = registry()
        reg["form"][-1]["shift_count_bits"] = 6
        self.rejects(reg, "S_ASHR_I32: shift_count_bits must be log2 of the width (32), got 6")

    def test_rejects_a_count_width_on_a_non_shift(self):
        reg = registry()
        reg["form"][0]["shift_count_bits"] = 5
        self.rejects(reg, "S_ADD_U32: shift_count_bits on a non-shift operation")

    def test_rejects_a_signed_shift_count_in_the_xml(self):
        snap = snapshot()
        snap["variants"][("S_ASHR_I32", "ENC_SOP2")]["operands"] = _operands(I32)
        self.rejects(registry(), "S_ASHR_I32: shift count SSRC1 has XML format FMT_NUM_I32", snap)

    def test_rejects_a_float_shift_count_in_the_xml(self):
        snap = snapshot()
        snap["variants"][("S_ASHR_I32", "ENC_SOP2")]["operands"] = _operands(
            I32, src1_format="FMT_NUM_F32"
        )
        self.rejects(registry(), "S_ASHR_I32: shift count SSRC1 has XML format FMT_NUM_F32", snap)

    def test_rejects_a_float_source_format(self):
        snap = snapshot()
        snap["variants"][("S_AND_B32", "ENC_SOP2")]["operands"] = _operands("FMT_NUM_F32")
        self.rejects(registry(), "S_AND_B32: signedness bitwise contradicts XML format FMT_NUM_F32", snap)

    def test_rejects_a_negative_count_width(self):
        reg = registry()
        reg["form"][-1]["shift_count_bits"] = -1
        self.rejects(reg, "S_ASHR_I32: shift_count_bits must be log2 of the width (32), got -1")

    def test_rejects_an_unsigned_arithmetic_shift(self):
        reg = registry()
        reg["form"][-1]["signedness"] = "unsigned"
        self.rejects(reg, "S_ASHR_I32: no primitive implements ashr/unsigned")

    def test_renders_the_declared_count_mask(self):
        self.assertIn(
            "Self::AshrI32 => primitive::ashr_nonzero_i32(src0, src1 & 0x1f),",
            gen.render(registry(), snapshot()),
        )

    def test_accepts_unaries(self):
        self.assertEqual(self.errors(unaries()), [])

    def test_rejects_a_sext_without_source_bits(self):
        reg = unaries()
        del reg["form"][-1]["source_bits"]
        self.rejects(reg, "S_SEXT_I32_I8: source_bits must be one of [8, 16], got None")

    def test_rejects_a_sext_whose_source_bits_contradict_the_xml(self):
        reg = unaries()
        reg["form"][-1]["source_bits"] = 16
        self.rejects(reg, "S_SEXT_I32_I8: SSRC0 is 8 bits in the XML, declared 16")
        self.rejects(reg, "S_SEXT_I32_I8: SSRC0 has XML format FMT_NUM_I8, not FMT_NUM_I16")

    def test_rejects_source_bits_on_another_operation(self):
        reg = unaries()
        reg["form"][-2]["source_bits"] = 8
        self.rejects(reg, "S_BREV_B32: source_bits on a non-sext operation")

    def test_rejects_an_scc_write_the_xml_does_not_show(self):
        reg = unaries()
        reg["form"][-2]["operation"] = "not"
        reg["form"][-2]["scc"] = "nonzero"
        self.rejects(reg, "S_BREV_B32: scc nonzero contradicts the XML SCC output")

    def test_rejects_a_unary_operation_under_a_binary_rule(self):
        reg = unaries()
        reg["form"][0]["operation"] = "not"
        reg["form"][0]["scc"] = "nonzero"
        self.rejects(reg, "S_ADD_U32: operation not does not fit rule sop2_sgpr")

    def test_renders_unary_scc_as_optional(self):
        text = gen.render(unaries(), snapshot())
        self.assertIn("Self::BrevB32 => (primitive::brev_b32(src), None),", text)
        self.assertIn("Self::SextI32I8 => (primitive::sext_i32(src, 8), None),", text)
        self.assertIn("let (value, scc) = primitive::not_nonzero_b32(src);", text)

    def test_accepts_compares(self):
        self.assertEqual(self.errors(compares()), [])

    def test_rejects_a_compare_without_a_relation(self):
        reg = compares()
        del reg["form"][-1]["relation"]
        self.rejects(reg, "S_CMP_LT_U32: unknown or missing relation None")

    def test_rejects_an_unknown_relation(self):
        reg = compares()
        reg["form"][-1]["relation"] = "ne"
        self.rejects(reg, "S_CMP_LT_U32: unknown or missing relation 'ne'")

    def test_rejects_a_non_string_relation(self):
        reg = compares()
        reg["form"][-1]["relation"] = ["lt"]
        self.rejects(reg, "S_CMP_LT_U32: unknown or missing relation ['lt']")

    def test_rejects_a_rule_without_sources(self):
        reg = compares()
        del reg["operand_rules"]["sopc_scalar"]["src"]
        self.rejects(reg, "operand rule sopc_scalar: declaration differs from the decoder's implemented rule")

    def test_rejects_a_relation_on_a_non_compare(self):
        reg = compares()
        reg["form"][0]["relation"] = "lt"
        self.rejects(reg, "S_ADD_U32: relation on a non-compare operation")

    def test_rejects_a_compare_with_another_scc_effect(self):
        reg = compares()
        reg["form"][-1]["scc"] = "nonzero"
        self.rejects(reg, "S_CMP_LT_U32: scc nonzero contradicts compare (requires relation)")

    def test_rejects_a_bitwise_compare(self):
        reg = compares()
        reg["form"][-1]["signedness"] = "bitwise"
        self.rejects(reg, "S_CMP_LT_U32: no primitive implements lt/bitwise")

    def test_rejects_a_compare_whose_signedness_contradicts_the_xml(self):
        reg = compares()
        reg["form"][-1]["signedness"] = "signed"
        self.rejects(reg, "S_CMP_LT_U32: signedness signed contradicts XML format FMT_NUM_U32")

    def test_rejects_a_compare_under_an_alu_rule(self):
        reg = compares()
        reg["form"][0]["operation"] = "compare"
        reg["form"][0]["relation"] = "lt"
        reg["form"][0]["scc"] = "relation"
        self.rejects(reg, "S_ADD_U32: operation compare does not fit rule sop2_sgpr")

    def test_renders_compare_dispatch_and_relation(self):
        text = gen.render(compares(), snapshot())
        self.assertIn("if word & 0xff80_0000 != 0xbf00_0000 {", text)
        self.assertIn("Self::CmpLtI32 => primitive::lt_i32(src0, src1),", text)
        self.assertIn("Self::CmpLtU32 => primitive::lt_u32(src0, src1),", text)
        self.assertIn("relation: Some(CompareRelation::Less),", text)

    def test_rejects_an_incomplete_unsupported_list(self):
        reg = registry()
        reg["operand_rules"]["sop2_sgpr"]["unsupported"].remove("two_literals")
        self.rejects(
            reg, "operand rule sop2_sgpr: unsupported combos differ from those the decoder rejects"
        )

    def test_rejects_a_citation_to_another_file(self):
        reg = registry()
        reg["documents"]["guide"]["file"] = "missing.pdf"
        self.rejects(reg, "document guide: differs from the pinned public document")

    def test_rejects_an_unenforced_wave_restriction(self):
        reg = registry()
        reg["form"][0]["wave"] = "wave32"
        self.rejects(reg, "S_ADD_U32: wave wave32 has no decoder enforcement yet")

    def test_rejects_a_reserved_variant(self):
        reg = registry()
        reg["form"][0]["variant"] = "Self"
        self.rejects(reg, "S_ADD_U32: variant 'Self' is reserved")

    def test_rejects_a_boolean_schema(self):
        reg = registry()
        reg["schema"] = True
        self.rejects(reg, "schema True is not 1")

    def test_rejects_unknown_unsupported_combo(self):
        reg = registry()
        reg["operand_rules"]["sop2_sgpr"]["unsupported"].append("dst_lds")
        self.rejects(reg, "operand rule sop2_sgpr: unknown unsupported combos ['dst_lds']")

    def test_rejects_duplicate_unsupported_combo(self):
        reg = registry()
        reg["operand_rules"]["sop2_sgpr"]["unsupported"].append("dst_vcc")
        self.rejects(reg, "operand rule sop2_sgpr: duplicate unsupported combos")

    # ---- remaining vocabulary, identity and reference rules ----

    def test_rejects_wrong_architecture(self):
        reg = registry()
        reg["architecture"] = "gfx1100"
        self.rejects(reg, "architecture 'gfx1100' is not gfx1030")

    def test_rejects_unknown_document(self):
        reg = registry()
        reg["documents"]["blog"] = {"title": "blog"}
        self.rejects(reg, "unknown document blog")

    def test_rejects_unknown_signedness_carry_in_exec_and_wave(self):
        for key, value in (
            ("signedness", "float"), ("carry_in", "vcc"), ("exec", "written"), ("wave", "wave64"),
        ):
            with self.subTest(key=key):
                reg = registry()
                form(reg, "S_ADD_U32")[key] = value
                self.rejects(reg, f"S_ADD_U32: unknown {key} {value!r}")

    def test_rejects_invalid_name_variant_and_opcode(self):
        for key, value, fragment in (
            ("name", "s_add_u32", "s_add_u32: invalid name"),
            ("variant", "add_u32", "S_ADD_U32: invalid variant 'add_u32'"),
            ("opcode", -1, "S_ADD_U32: invalid opcode"),
            ("opcode", True, "S_ADD_U32: invalid opcode"),
        ):
            with self.subTest(key=key, value=value):
                reg = registry()
                form(reg, "S_ADD_U32")[key] = value
                self.rejects(reg, fragment)

    def test_rejects_declaration_no_primitive_implements(self):
        reg = registry()
        form(reg, "S_AND_B32")["operation"] = "sub"
        self.rejects(reg, "S_AND_B32: no primitive implements sub/bitwise")

    def test_rejects_duplicate_evidence(self):
        reg = registry()
        form(reg, "S_ADD_U32")["verify"] = ["reference", "reference"]
        self.rejects(reg, "S_ADD_U32: duplicate verification requirement")

    def test_rejects_malformed_spec_references(self):
        for ref, fragment in (
            ({"document": "guide", "section": "12.1"}, "spec reference needs exactly document, section, page"),
            ({"document": "guide", "section": "12.x", "page": 101}, "invalid spec section '12.x'"),
            ({"document": "guide", "section": "12.1", "page": -1}, "invalid spec page"),
        ):
            with self.subTest(ref=ref):
                reg = registry()
                form(reg, "S_ADD_U32")["spec"].append(ref)
                self.rejects(reg, f"S_ADD_U32: {fragment}")

    def test_rejects_guide_reference_without_a_page(self):
        reg = registry()
        form(reg, "S_ADD_U32")["spec"] = [{"document": "guide", "section": "12.1", "page": 0}]
        self.rejects(reg, "S_ADD_U32: missing a guide section and page reference")

    def test_rejects_opcode_outside_the_encoding_identifiers(self):
        snap = snapshot()
        snap["encodings"]["ENC_SOP2"]["identifiers"].remove(0x80000000 | (14 << 23))
        self.rejects(registry(), "S_AND_B32: opcode 14 is not an identifier of ENC_SOP2", snap)

    def test_rejects_xml_operand_disagreements(self):
        cases = (
            ("missing", lambda ops: ops.pop(2), "S_ADD_U32: XML has no operand SSRC1"),
            ("width", lambda ops: ops[1].__setitem__(3, 64), "S_ADD_U32: SSRC0 is 64 bits in the XML"),
            ("direction", lambda ops: ops[0].__setitem__(4, True),
             "S_ADD_U32: SDST direction disagrees with the XML"),
            ("extra", lambda ops: ops.append(["SIMM16", U32, "OPR_SIMM16", 16, True, False, False, True]),
             "S_ADD_U32: XML operands ['SIMM16'] are not covered by rule sop2_sgpr"),
        )
        for label, change, fragment in cases:
            with self.subTest(case=label):
                snap = snapshot()
                change(snap["variants"][("S_ADD_U32", "ENC_SOP2")]["operands"])
                self.rejects(registry(), fragment, snap)


def _reorder(value):
    """Reverse the key order of every mapping, keeping list order."""
    if isinstance(value, dict):
        return {key: _reorder(value[key]) for key in reversed(list(value))}
    if isinstance(value, list):
        return [_reorder(item) for item in value]
    return value


class Render(unittest.TestCase):
    def test_two_renders_are_identical(self):
        self.assertEqual(gen.render(registry(), snapshot()), gen.render(registry(), snapshot()))

    def test_render_ignores_mapping_key_order(self):
        reg = registry()
        reordered = _reorder(reg)
        self.assertNotEqual(list(reg["form"][0]), list(reordered["form"][0]))
        self.assertEqual(gen.render(reg, snapshot()), gen.render(reordered, snapshot()))

    def test_dispatch_table_is_indexed_by_opcode(self):
        reg = registry()
        reg["form"].reverse()
        text = gen.render(reg, snapshot())
        body = text.split("const SCALAR_ALU_OP_BY_OPCODE: [Option<ScalarAluOp>; 128] = [", 1)[1].split("];", 1)[0]
        rows = [line.strip() for line in body.splitlines() if line.strip()]
        self.assertEqual(len(rows), 128)
        declared = {index: row for index, row in enumerate(rows) if row != "None,"}
        self.assertEqual(sorted(declared), [0, 2, 4, 14, 34])
        self.assertEqual(declared[34], "Some(ScalarAluOp::AshrI32),")
        self.assertIn("if word & 0xc000_0000 != 0x8000_0000 {", text)
        self.assertIn("SCALAR_ALU_OP_BY_OPCODE[((word >> 23) & 0x7f) as usize]", text)

    def test_rejects_identifiers_that_do_not_share_the_encoding_bits(self):
        snap = snapshot()
        snap["encodings"]["ENC_SOP2"]["identifiers"].append(0x40000000)
        with self.assertRaisesRegex(ValueError, "do not split into fixed bits and OP"):
            gen.render(registry(), snap)

    def test_real_registry_renders_the_committed_file(self):
        text = gen.render(gen.load_registry(), gen.load_snapshot())
        self.assertEqual(text, gen.RUST.read_text(encoding="utf-8"))


class CommandLine(unittest.TestCase):
    def run_cli(self, *arguments):
        return subprocess.run(
            [sys.executable, str(SCRIPT), *map(str, arguments)],
            capture_output=True, text=True, check=False,
        )

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.dir = Path(self.tmp.name)

    def test_check_passes_on_the_committed_file(self):
        result = self.run_cli("--check")
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_check_passes_on_a_fresh_file_and_fails_on_a_stale_one(self):
        output = self.dir / "semantics_gen.rs"
        result = self.run_cli("generate", "--output", output)
        self.assertEqual(result.returncode, 0, result.stderr)
        fresh = output.read_text(encoding="utf-8")
        self.assertEqual(self.run_cli("--check", "--output", output).returncode, 0)

        output.write_text(fresh.replace("pub const GENERATED_SCHEMA: u32 = 1;", "pub const GENERATED_SCHEMA: u32 = 2;"))
        stale = self.run_cli("--check", "--output", output)
        self.assertEqual(stale.returncode, 1)
        self.assertIn("is stale", stale.stderr)
        # --check never writes.
        self.assertNotEqual(output.read_text(encoding="utf-8"), fresh)

    def test_check_fails_when_the_generated_file_is_missing(self):
        self.assertEqual(self.run_cli("--check", "--output", self.dir / "absent.rs").returncode, 1)

    def test_invalid_registry_fails_every_mode_without_writing(self):
        bad = self.dir / "rdna2.toml"
        text = gen.REGISTRY.read_text(encoding="utf-8")
        bad.write_text(text.replace("\nschema = 1\n", "\nschema = 2\n", 1), encoding="utf-8")
        output = self.dir / "out.rs"
        for mode in (("validate",), ("generate",), ("--check",)):
            result = self.run_cli(*mode, "--registry", bad, "--output", output)
            self.assertEqual(result.returncode, 1, mode)
            self.assertIn("error: schema 2 is not 1", result.stderr)
        self.assertFalse(output.exists())

    def test_validate_reports_the_form_count(self):
        result = self.run_cli("validate")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout.strip(), f"registry valid: {len(gen.load_registry()['form'])} forms")


if __name__ == "__main__":
    unittest.main()
