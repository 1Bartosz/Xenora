# SPDX-License-Identifier: GPL-3.0-or-later
"""Authoritative x86-64 inventory from the instruction registry.

Consumes `crates/loader/x86/instructions.toml` via `generate_x86.load`.
Classifies native lowering from read-only `is_native`/`neon_operation` arms in
`dbt.rs`. Does not modify CPU crates or invent support percentages.
"""
from __future__ import annotations

import json
import re
import runpy
import sys
from collections import defaultdict
from dataclasses import asdict, dataclass, field
from pathlib import Path
from typing import Any


REGISTRY_REL = Path("crates/loader/x86/instructions.toml")
GENERATOR_REL = Path("crates/loader/generate_x86.py")
DBT_REL = Path("crates/loader/src/dbt.rs")
FIXTURE_RELS = (
    Path("crates/loader/tests/x86/integer.txt"),
    Path("crates/loader/tests/x86/vector.txt"),
    Path("crates/loader/tests/x86/float.txt"),
)

# Documented intentional rejections (docs/cpu/instruction-registry.md).
# Not registry forms; tracked separately so they are never counted as declared.
UNSUPPORTED_CATALOG = (
    "LOCK",
    "F2/F3-as-rep (except PAUSE / declared forms)",
    "F2+F3 combined",
    "address-size override",
    "ignored segment after FS/GS",
    "16-bit stack/branch",
    "16-bit BSWAP/MOVSXD",
    "BT memory by register index",
    "RCL/RCR/SHLD/SHRD",
    "string instructions",
    "MMX",
    "RCP/RSQRT approximations",
    "SSE3+",
    "VEX",
    "atomics/fences",
    "raw SYSCALL in DBT",
)


@dataclass(frozen=True)
class FormRecord:
    op: str
    map: str
    prefix: str
    opcode: int
    digit: int | None
    enc: str
    size: str
    imm: str
    src: str
    expanded_from_count: bool = False


@dataclass
class CpuInventory:
    schema: int
    architecture: str
    form_count: int
    operation_count: int
    by_map: dict[str, int] = field(default_factory=dict)
    by_prefix: dict[str, int] = field(default_factory=dict)
    by_size: dict[str, int] = field(default_factory=dict)
    by_enc: dict[str, int] = field(default_factory=dict)
    operations: list[str] = field(default_factory=list)
    native_ops: list[str] = field(default_factory=list)
    shared_semantic_ops: list[str] = field(default_factory=list)
    fixture_mnemonics: list[str] = field(default_factory=list)
    ops_missing_fixture_evidence: list[str] = field(default_factory=list)
    unsupported_catalog: list[str] = field(default_factory=list)
    forms: list[dict[str, Any]] = field(default_factory=list)
    notes: list[str] = field(default_factory=list)

    def to_dict(self) -> dict[str, Any]:
        return asdict(self)


def _load_generator(root: Path):
    path = root / GENERATOR_REL
    if not path.is_file():
        raise FileNotFoundError(f"missing generator {path}")
    # Keep generate_x86 importable without installing the package.
    sys.path.insert(0, str(path.parent))
    try:
        return runpy.run_path(str(path))
    finally:
        if sys.path and sys.path[0] == str(path.parent):
            sys.path.pop(0)


def expand_forms(raw_forms: list[dict[str, Any]]) -> list[FormRecord]:
    """Expand count>1 encodings into per-opcode records (matches generator slots)."""
    out: list[FormRecord] = []
    for form in raw_forms:
        count = int(form.get("count", 1))
        base = int(form["opcode"])
        for offset in range(count):
            out.append(
                FormRecord(
                    op=str(form["op"]),
                    map=str(form.get("map", "1")),
                    prefix=str(form.get("prefix", "") or ""),
                    opcode=base + offset,
                    digit=form.get("digit"),
                    enc=str(form["enc"]),
                    size=str(form["size"]),
                    imm=str(form.get("imm", "") or ""),
                    src=str(form.get("src", "") or ""),
                    expanded_from_count=count > 1,
                )
            )
    return out


def parse_native_ops(dbt_text: str) -> set[str]:
    """Collect Op::* names that appear as unconditional native arms or NEON table keys.

    Conditional native (wide-only, encoding-gated) still counts the op as having
    a native lowering path; shared-semantic remains available for other widths.
    """
    native: set[str] = set()
    # Arms of is_native before the final `_ => false`
    fn = re.search(r"pub fn is_native\(.*?\{(.*?)\n\}", dbt_text, re.S)
    body = fn.group(1) if fn else dbt_text
    for match in re.finditer(r"Op::([A-Za-z0-9_]+)", body):
        # Stop once we hit neon_operation definition if we grabbed too much.
        native.add(_rust_op_to_registry(match.group(1)))
    neon = re.search(r"fn neon_operation\(.*?\{(.*?)\n\}", dbt_text, re.S)
    if neon:
        for match in re.finditer(r"Op::([A-Za-z0-9_]+)", neon.group(1)):
            native.add(_rust_op_to_registry(match.group(1)))
    native.discard("syscall")  # is_translatable rejects; not a native lower
    return native


def _rust_op_to_registry(name: str) -> str:
    # Op::Movsxd -> movsxd, Op::Add -> add
    out = []
    for i, ch in enumerate(name):
        if ch.isupper() and i:
            out.append("_")
        out.append(ch.lower())
    return "".join(out).replace("__", "_")


def parse_fixture_mnemonics(texts: list[str]) -> set[str]:
    """First token after `|` in `hex | asm` fixture lines."""
    found: set[str] = set()
    for text in texts:
        for line in text.splitlines():
            if " | " not in line:
                continue
            asm = line.split(" | ", 1)[1].strip()
            if not asm:
                continue
            token = asm.split()[0].lower()
            # strip width suffixes used in intel syntax occasionally
            found.add(token)
    return found


def mnemonic_covers_op(mnemonic: str, op: str) -> bool:
    if mnemonic == op:
        return True
    # Common aliases: jcc conditions, cmovcc, setcc, imul2 -> imul
    if op == "jcc" and mnemonic.startswith("j") and mnemonic not in {"jmp", "jecxz", "jrcxz"}:
        return True
    if op == "cmov" and mnemonic.startswith("cmov"):
        return True
    if op == "setcc" and mnemonic.startswith("set"):
        return True
    if op == "imul2" and mnemonic == "imul":
        return True
    if op in {"cwde", "cdq"} and mnemonic in {"cwde", "cdqe", "cdq", "cqo", "cbw", "cwd"}:
        return True
    return False


def build_inventory(root: Path) -> CpuInventory:
    gen = _load_generator(root)
    text = (root / REGISTRY_REL).read_text(encoding="utf-8")
    operations, forms = gen["load"](text)
    records = expand_forms(forms)
    by_map: dict[str, int] = defaultdict(int)
    by_prefix: dict[str, int] = defaultdict(int)
    by_size: dict[str, int] = defaultdict(int)
    by_enc: dict[str, int] = defaultdict(int)
    for rec in records:
        by_map[rec.map] += 1
        by_prefix[rec.prefix or "none"] += 1
        by_size[rec.size] += 1
        by_enc[rec.enc] += 1

    dbt = (root / DBT_REL).read_text(encoding="utf-8") if (root / DBT_REL).is_file() else ""
    native_ops = sorted(parse_native_ops(dbt) & set(operations))
    shared = sorted(set(operations) - set(native_ops))

    fixture_texts = []
    for rel in FIXTURE_RELS:
        path = root / rel
        if path.is_file():
            fixture_texts.append(path.read_text(encoding="utf-8"))
    mnemonics = parse_fixture_mnemonics(fixture_texts)
    missing = sorted(
        op for op in operations
        if not any(mnemonic_covers_op(m, op) for m in mnemonics)
        and op not in {"ldmxcsr", "stmxcsr", "jcc", "jmp", "call", "ret", "hlt", "syscall"}
    )

    notes = [
        "Declared forms are decodable; support requires semantics + fixtures (docs/cpu/instruction-registry.md).",
        "Native ops are read from dbt.rs is_native/neon_operation; conditional width/encoding gates still use shared semantics for other cases.",
        "Unsupported catalog is documentation of intentional rejections, not missing registry rows.",
        "Does not inflate coverage: fixture evidence is mnemonic-level, not form-index proof.",
    ]
    return CpuInventory(
        schema=1,
        architecture="x86-64",
        form_count=len(records),
        operation_count=len(operations),
        by_map=dict(sorted(by_map.items())),
        by_prefix=dict(sorted(by_prefix.items())),
        by_size=dict(sorted(by_size.items())),
        by_enc=dict(sorted(by_enc.items())),
        operations=sorted(operations),
        native_ops=native_ops,
        shared_semantic_ops=shared,
        fixture_mnemonics=sorted(mnemonics),
        ops_missing_fixture_evidence=missing,
        unsupported_catalog=list(UNSUPPORTED_CATALOG),
        forms=[asdict(r) for r in records],
        notes=notes,
    )


def compact_inventory(inv: CpuInventory) -> str:
    return (
        f"forms={inv.form_count} ops={inv.operation_count} "
        f"native={len(inv.native_ops)} shared={len(inv.shared_semantic_ops)} "
        f"maps={inv.by_map} unsupported_catalog={len(inv.unsupported_catalog)}\n"
    )


def dumps_inventory(inv: CpuInventory, *, include_forms: bool = False) -> str:
    data = inv.to_dict()
    if not include_forms:
        data.pop("forms", None)
    return json.dumps(data, indent=2, sort_keys=True) + "\n"
