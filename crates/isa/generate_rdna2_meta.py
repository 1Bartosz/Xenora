#!/usr/bin/env python3
"""Generate the complete gfx1030 instruction metadata table.

Two stages keep regeneration offline and checkable without the 11 MB AMD XML:

  snapshot  AMD RDNA 2 XML -> crates/isa/data/rdna2.jsonl (normalised, sorted)
  rust      crates/isa/data/rdna2.jsonl -> crates/isa/src/rdna2_meta.rs

`--check` regenerates in memory and fails when a committed file differs.
Only the Python standard library is used. See docs/research/rdna2.md.
"""

import argparse
import hashlib
import json
import sys
from pathlib import Path
from xml.etree import ElementTree

SOURCE_SHA256 = "d671ecbc36543674ab59e9b2feffd56718fd2137e7ad913c314f2e2508b9f4f7"
ARCHITECTURE = "AMD RDNA 2"
ROOT = Path(__file__).resolve().parents[2]
SNAPSHOT = ROOT / "crates/isa/data/rdna2.jsonl"
RUST = ROOT / "crates/isa/src/rdna2_meta.rs"
SCHEMA = 1

# Encodings whose default condition defines an instruction's base form. The
# remaining XML encodings are literal, SDWA, DPP or NSA extensions of these.
BASE_ENCODINGS = {
    "ENC_SOP1": "Sop1",
    "ENC_SOP2": "Sop2",
    "ENC_SOPC": "Sopc",
    "ENC_SOPK": "Sopk",
    "ENC_SOPP": "Sopp",
    "ENC_SMEM": "Smem",
    "ENC_VOP1": "Vop1",
    "ENC_VOP2": "Vop2",
    "ENC_VOPC": "Vopc",
    "ENC_VOP3": "Vop3",
    "VOP3_SDST_ENC": "Vop3Sdst",
    "ENC_VOP3P": "Vop3p",
    "ENC_VINTRP": "Vintrp",
    "ENC_DS": "Ds",
    "ENC_MUBUF": "Mubuf",
    "ENC_MTBUF": "Mtbuf",
    "ENC_MIMG": "Mimg",
    "ENC_EXP": "Exp",
    "ENC_FLAT": "Flat",
    "ENC_FLAT_GLBL": "Global",
    "ENC_FLAT_SCRATCH": "Scratch",
}

# Extension encoding -> (base encoding, extension kind). The XML names the
# variant and its operands but gives no expression for when it applies; the
# decoder implements those rules by hand (docs/research/rdna2.md).
EXTENSIONS = {
    "SOP1_INST_LITERAL": ("ENC_SOP1", "literal"),
    "SOP2_INST_LITERAL": ("ENC_SOP2", "literal"),
    "SOPC_INST_LITERAL": ("ENC_SOPC", "literal"),
    "SOPK_INST_LITERAL": ("ENC_SOPK", "literal"),
    "VOP1_INST_LITERAL": ("ENC_VOP1", "literal"),
    "VOP2_INST_LITERAL": ("ENC_VOP2", "literal"),
    "VOPC_INST_LITERAL": ("ENC_VOPC", "literal"),
    "VOP3_INST_LITERAL": ("ENC_VOP3", "literal"),
    "VOP3_SDST_ENC_INST_LITERAL": ("VOP3_SDST_ENC", "literal"),
    "VOP3P_INST_LITERAL": ("ENC_VOP3P", "literal"),
    "VOP1_VOP_SDWA": ("ENC_VOP1", "sdwa"),
    "VOP2_VOP_SDWA": ("ENC_VOP2", "sdwa"),
    "VOP2_VOP_SDWA_SDST_ENC": ("ENC_VOP2", "sdwa"),
    "VOPC_VOP_SDWA_SDST_ENC": ("ENC_VOPC", "sdwa"),
    "VOP1_VOP_DPP8": ("ENC_VOP1", "dpp8"),
    "VOP2_VOP_DPP8": ("ENC_VOP2", "dpp8"),
    "VOP1_VOP_DPP16": ("ENC_VOP1", "dpp16"),
    "VOP2_VOP_DPP16": ("ENC_VOP2", "dpp16"),
    "MIMG_NSA1": ("ENC_MIMG", "nsa"),
    "MIMG_NSA2": ("ENC_MIMG", "nsa"),
    "MIMG_NSA3": ("ENC_MIMG", "nsa"),
}

FLAGS = ("IsBranch", "IsConditionalBranch", "IsIndirectBranch", "IsProgramTerminator", "IsImmediatelyExecuted")


def fail(message):
    raise SystemExit(f"generate_rdna2_meta: {message}")


def text(element, path):
    value = element.findtext(path)
    if value is None:
        fail(f"missing <{path}>")
    return value.strip()


def boolean(value):
    if value not in ("TRUE", "FALSE", "true", "false"):
        fail(f"unexpected boolean {value!r}")
    return value.lower() == "true"


def snapshot_from_xml(data):
    if hashlib.sha256(data).hexdigest() != SOURCE_SHA256:
        fail("RDNA 2 XML does not match the approved input SHA-256")
    root = ElementTree.fromstring(data)
    document = root.find("Document")
    isa = root.find("ISA")
    if text(isa, "Architecture/ArchitectureName") != ARCHITECTURE:
        fail("XML is not the RDNA 2 architecture")
    records = [{
        "kind": "source",
        "schema": SCHEMA,
        "architecture": ARCHITECTURE,
        "sha256": SOURCE_SHA256,
        "release": text(document, "ReleaseDate"),
        "xml_schema": text(document, "SchemaVersion"),
        "license": text(document, "License"),
        "copyright": text(document, "Copyright"),
    }]

    known = set(BASE_ENCODINGS) | set(EXTENSIONS)
    encodings = {}
    for encoding in isa.findall("Encodings/Encoding"):
        name = text(encoding, "EncodingName")
        if name not in known:
            fail(f"unclassified encoding {name}; add it to BASE_ENCODINGS or EXTENSIONS")
        bits = int(text(encoding, "BitCount"))
        mask = int(text(encoding, "EncodingIdentifierMask"), 2)
        identifiers = sorted(int(item.text, 2) for item in encoding.findall("EncodingIdentifiers/EncodingIdentifier"))
        if mask >> 32 or any(identifier >> 32 for identifier in identifiers):
            fail(f"{name} identifies instructions beyond the first dword")
        fields = []
        for field in encoding.findall("MicrocodeFormat/BitMap/Field"):
            ranges = sorted(field.findall("BitLayout/Range"), key=lambda item: int(item.attrib.get("Order", 0)))
            fields.append([
                text(field, "FieldName"),
                [[int(text(item, "BitOffset")), int(text(item, "BitCount"))] for item in ranges],
            ])
        fields.sort(key=lambda item: (item[1][0][0], item[0]))
        encodings[name] = {"mask": mask, "identifiers": set(identifiers)}
        records.append({
            "kind": "encoding",
            "name": name,
            "bits": bits,
            "identifier_mask": mask,
            "identifiers": identifiers,
            "conditions": sorted(text(item, "ConditionName") for item in encoding.findall("EncodingConditions/EncodingCondition")),
            "fields": fields,
        })
    missing = known - set(encodings)
    if missing:
        fail(f"encodings absent from the XML: {sorted(missing)}")

    for operand_type in isa.findall("OperandTypes/OperandType"):
        values = sorted(
            [text(value, "Name"), int(text(value, "Value"))]
            for value in operand_type.findall("OperandPredefinedValues/PredefinedValue")
        )
        records.append({
            "kind": "operand_type",
            "name": text(operand_type, "OperandTypeName"),
            "partitioned": boolean(operand_type.attrib.get("IsPartitioned", "false")),
            "values": values,
        })

    for data_format in isa.findall("DataFormats/DataFormat"):
        records.append({
            "kind": "data_format",
            "name": text(data_format, "DataFormatName"),
            "data_type": text(data_format, "DataType"),
            "bits": int(text(data_format, "BitCount")),
            "components": int(text(data_format, "ComponentCount")),
        })

    seen = set()
    for instruction in isa.findall("Instructions/Instruction"):
        name = text(instruction, "InstructionName")
        flags = {flag: boolean(text(instruction, f"InstructionFlags/{flag}")) for flag in FLAGS}
        group = instruction.find("FunctionalGroup")
        for variant in instruction.findall("InstructionEncodings/InstructionEncoding"):
            encoding_name = text(variant, "EncodingName")
            condition = text(variant, "EncodingCondition")
            opcode = int(text(variant, "Opcode"))
            key = (name, encoding_name, condition)
            if key in seen:
                fail(f"duplicate variant {key}")
            seen.add(key)
            operands = []
            for operand in sorted(variant.findall("Operands/Operand"), key=lambda item: int(item.attrib["Order"])):
                operands.append([
                    operand.findtext("FieldName") or "",
                    operand.findtext("DataFormatName") or "",
                    operand.findtext("OperandType") or "",
                    int(operand.findtext("OperandSize") or 0),
                    boolean(operand.attrib["Input"]),
                    boolean(operand.attrib["Output"]),
                    boolean(operand.attrib["IsImplicit"]),
                    boolean(operand.attrib.get("IsBinaryMicrocodeRequired", "true")),
                ])
            record = {
                "kind": "variant",
                "instruction": name,
                "encoding": encoding_name,
                "condition": condition,
                "opcode": opcode,
            }
            # Extension variants keep only what classifies them; their
            # operands repeat the base form with a literal, SDWA, DPP or NSA word.
            if encoding_name in BASE_ENCODINGS or condition == "default":
                record.update({
                    "flags": sorted(flag for flag, value in flags.items() if value),
                    "group": text(group, "Name"),
                    "subgroups": sorted(item.text.strip() for item in group.findall("FunctionalSubgroups/Subgroup")),
                    "operands": operands,
                })
            records.append(record)

    order = {"source": 0, "encoding": 1, "operand_type": 2, "data_format": 3, "variant": 4}
    records.sort(key=lambda record: (
        order[record["kind"]],
        record.get("name", ""),
        record.get("encoding", ""),
        record.get("instruction", ""),
        record.get("condition", ""),
    ))
    return "".join(json.dumps(record, sort_keys=True, separators=(",", ":")) + "\n" for record in records)


def place(value, ranges):
    """Places `value` into a field layout; returns (mask, bits) within the first two dwords."""
    mask = bits = 0
    for offset, count in ranges:
        if offset + count > 64:
            fail("opcode field extends beyond the second dword")
        low = (1 << count) - 1
        bits |= (value & low) << offset
        mask |= low << offset
        value >>= count
    if value:
        fail("opcode does not fit its field")
    return mask, bits


def forms_from_snapshot(snapshot):
    records = [json.loads(line) for line in snapshot.splitlines()]
    source = records[0]
    if source.get("kind") != "source" or source.get("schema") != SCHEMA or source.get("sha256") != SOURCE_SHA256:
        fail("snapshot header does not match this generator")
    encodings = {record["name"]: record for record in records if record["kind"] == "encoding"}
    variants = [record for record in records if record["kind"] == "variant"]

    extensions = {}
    for variant in variants:
        if variant["encoding"] in EXTENSIONS:
            base, kind = EXTENSIONS[variant["encoding"]]
            entry = extensions.setdefault((variant["instruction"], base), set())
            if kind == "literal" and variant["condition"] == "default":
                entry.add("always_literal")
            entry.add(kind)

    based = {(variant["instruction"], variant["encoding"]) for variant in variants if variant["encoding"] in BASE_ENCODINGS}
    forms = []
    for variant in variants:
        encoding_name = variant["encoding"]
        identity = encoding_name
        if encoding_name in EXTENSIONS:
            # A few instructions exist only with a literal (S_SETREG_IMM32_B32,
            # V_FMAMK_F32, ...): the XML lists them under the literal encoding's
            # default condition and gives them no base variant.
            base, kind = EXTENSIONS[encoding_name]
            if kind != "literal" or variant["condition"] != "default" or (variant["instruction"], base) in based:
                continue
            encoding_name = base
        elif encoding_name not in BASE_ENCODINGS:
            continue
        # Three VOP1/VOP3 relative moves name the excluded extensions instead of "default".
        if variant["condition"] != "default" and not variant["condition"].startswith("Nothas_"):
            fail(f"{variant['instruction']} has non-default base condition {variant['condition']}")
        encoding = encodings[encoding_name]
        id_encoding = encodings[identity]
        fields = dict((name, ranges) for name, ranges in encoding["fields"])
        prefix_mask, _ = place(0, fields["ENCODING"])
        prefixes = {identifier & prefix_mask for identifier in id_encoding["identifiers"]}
        if len(prefixes) != 1:
            fail(f"{encoding_name} has an ambiguous encoding prefix")
        prefix = prefixes.pop()
        id_mask = id_encoding["identifier_mask"] & 0xFFFFFFFF
        opcode = variant["opcode"]
        if "OP" in fields:
            op_mask, op_bits = place(opcode, fields["OP"])
        elif opcode == 0:
            op_mask = op_bits = 0
        else:
            fail(f"{encoding_name} has no OP field for opcode {opcode}")
        # The identifier mask can add selector bits beyond ENCODING and OP,
        # such as the FLAT segment field. Take them from the listed identifier.
        # MTBUF keeps one opcode bit in the second dword, which the
        # first-dword identifiers cannot express; carry it separately.
        low = 0xFFFFFFFF
        candidates = sorted({
            identifier & 0xFFFFFFFF for identifier in id_encoding["identifiers"]
            if identifier & (prefix_mask | op_mask) & low == (prefix | op_bits) & low
        })
        if len(candidates) != 1:
            fail(f"{variant['instruction']} matches {len(candidates)} {encoding_name} identifiers")
        forms.append({
            "name": variant["instruction"],
            "encoding": BASE_ENCODINGS[encoding_name],
            "opcode": opcode,
            # Opcode bits outside the identifier mask (MIMG bit 0, MTBUF bit 53)
            # still select the instruction.
            "mask": id_mask | op_mask,
            "bits": (candidates[0] & id_mask) | op_bits,
            "bytes": encoding["bits"] // 8,
            "flags": variant["flags"],
            "group": variant["group"],
            "extensions": sorted(extensions.get((variant["instruction"], encoding_name), ())),
            "operands": variant["operands"],
        })

    by_pattern = {}
    for form in forms:
        if form["bits"] & ~form["mask"]:
            fail(f"{form['name']} has identifying bits outside its mask")
        key = (form["mask"], form["bits"])
        if key in by_pattern:
            fail(f"{form['name']} and {by_pattern[key]['name']} share an encoding")
        by_pattern[key] = form
    for left in forms:
        for right in forms:
            if left is right or left["mask"] == right["mask"]:
                continue
            common = left["mask"] & right["mask"]
            if (left["bits"] ^ right["bits"]) & common == 0:
                fail(f"{left['name']} ({left['encoding']}) overlaps {right['name']} ({right['encoding']})")
    forms.sort(key=lambda form: (form["encoding"], form["opcode"]))
    return source, encodings, forms


def rust_from_snapshot(snapshot):
    source, encodings, forms = forms_from_snapshot(snapshot)
    out = []
    emit = out.append
    emit("// Generated by crates/isa/generate_rdna2_meta.py from crates/isa/data/rdna2.jsonl. Do not edit.")
    emit(f"// Source: AMD RDNA 2 XML, {source['release']}, schema {source['xml_schema']}, SHA-256 {source['sha256']}.")
    emit(f"// {source['copyright']}")
    emit("// SPDX-License-Identifier: MIT")
    emit("")
    emit("use super::meta::{EncodingFamily, Extensions, FieldLayout, Form, FormFlags, OperandInfo};")
    emit("")
    emit(f'/// SHA-256 of the AMD XML the table was generated from.\npub const SOURCE_SHA256: &str = "{source["sha256"]}";')
    emit("")

    operand_sets = {}
    for form in forms:
        operand_sets.setdefault(json.dumps(form["operands"]), len(operand_sets))
    emit("static OPERANDS: [&[OperandInfo]; %d] = [" % len(operand_sets))
    for operands_json in operand_sets:
        operands = json.loads(operands_json)
        items = ", ".join(
            "OperandInfo { field: %s, format: %s, kind: %s, bits: %d, input: %s, output: %s, implicit: %s, encoded: %s }"
            % (json.dumps(field), json.dumps(data_format), json.dumps(kind), size,
               str(i).lower(), str(o).lower(), str(implicit).lower(), str(encoded).lower())
            for field, data_format, kind, size, i, o, implicit, encoded in operands
        )
        emit(f"    &[{items}],")
    emit("];")
    emit("")

    families = sorted({form["encoding"] for form in forms})
    emit("/// Field layouts of each base encoding, as (bit offset, bit count) pieces from least significant.")
    emit("pub static FIELDS: [(EncodingFamily, &[FieldLayout]); %d] = [" % len(families))
    reverse = {value: key for key, value in BASE_ENCODINGS.items()}
    for family in families:
        fields = encodings[reverse[family]]["fields"]
        items = ", ".join(
            "FieldLayout { name: %s, pieces: &[%s] }" % (json.dumps(name), ", ".join(f"({o}, {c})" for o, c in ranges))
            for name, ranges in fields
        )
        emit(f"    (EncodingFamily::{family}, &[{items}]),")
    emit("];")
    emit("")

    flag_bits = {
        "IsBranch": 1,
        "IsConditionalBranch": 2,
        "IsIndirectBranch": 4,
        "IsProgramTerminator": 8,
        "IsImmediatelyExecuted": 16,
    }
    extension_bits = {"literal": 1, "always_literal": 2, "sdwa": 4, "dpp8": 8, "dpp16": 16, "nsa": 32}
    emit("/// Every gfx1030 base instruction form in the AMD XML, sorted by encoding family and opcode.")
    emit("pub static FORMS: [Form; %d] = [" % len(forms))
    for form in forms:
        flags = sum(flag_bits[flag] for flag in form["flags"])
        extensions = sum(extension_bits[item] for item in form["extensions"])
        emit(
            "    Form { name: %s, family: EncodingFamily::%s, opcode: %d, mask: 0x%016x, bits: 0x%016x, bytes: %d, "
            "flags: FormFlags(0x%02x), extensions: Extensions(0x%02x), group: %s, operands: OPERANDS[%d] },"
            % (json.dumps(form["name"]), form["encoding"], form["opcode"], form["mask"], form["bits"], form["bytes"],
               flags, extensions, json.dumps(form["group"]), operand_sets[json.dumps(form["operands"])])
        )
    emit("];")
    return "\n".join(out) + "\n"


def compare(path, expected):
    actual = path.read_text() if path.exists() else ""
    if actual != expected:
        print(f"{path.relative_to(ROOT)} is out of date; rerun crates/isa/generate_rdna2_meta.py", file=sys.stderr)
        return False
    return True


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--xml", type=Path, help="approved amdgpu_isa_rdna2.xml; regenerates the snapshot too")
    parser.add_argument("--check", action="store_true", help="fail when committed outputs differ")
    args = parser.parse_args()

    if args.xml:
        snapshot = snapshot_from_xml(args.xml.read_bytes())
    elif SNAPSHOT.exists():
        snapshot = SNAPSHOT.read_text()
    else:
        fail("no snapshot; pass --xml")
    rust = rust_from_snapshot(snapshot)
    if args.check:
        ok = compare(RUST, rust)
        if args.xml:
            ok = compare(SNAPSHOT, snapshot) and ok
        raise SystemExit(0 if ok else 1)
    SNAPSHOT.parent.mkdir(parents=True, exist_ok=True)
    SNAPSHOT.write_text(snapshot)
    RUST.write_text(rust)


if __name__ == "__main__":
    main()
