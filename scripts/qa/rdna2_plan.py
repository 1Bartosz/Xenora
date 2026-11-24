# SPDX-License-Identifier: GPL-3.0-or-later
"""Read-only RDNA2 expansion planning from existing inventories.

Reuses scripts/audit/semantic_coverage.py loaders and the checked-in AMD
snapshot, semantic registry, generated tables and status evidence. Does not
edit GPU production code. Candidate declarations are never counted as
implemented forms.
"""
from __future__ import annotations

import json
import runpy
import tomllib
from collections import defaultdict
from dataclasses import asdict, dataclass, field
from pathlib import Path
from typing import Any


def _load_coverage_module(root: Path):
    path = root / "scripts" / "audit" / "semantic_coverage.py"
    return runpy.run_path(str(path))


@dataclass(frozen=True)
class FormRef:
    family: str
    name: str
    opcode: int

    @property
    def key(self) -> tuple[str, str]:
        return (self.family, self.name)


@dataclass
class Candidate:
    family: str
    name: str
    opcode: int
    width: int | None
    numeric_type: str | None
    operand_layout: str | None
    reusable_primitives: list[str] = field(default_factory=list)
    missing_operand_rules: list[str] = field(default_factory=list)
    missing_effects: list[str] = field(default_factory=list)
    spec_refs: list[dict[str, Any]] = field(default_factory=list)
    priority_batch: str = "unclassified"
    notes: list[str] = field(default_factory=list)


@dataclass
class Inventory:
    identified: int
    declared: int
    decoder_supported: int
    reference_verified: int
    metal_verified: int
    unsupported: int
    by_family: dict[str, dict[str, int]]


def load_inventories(root: Path) -> dict[str, Any]:
    sc = _load_coverage_module(root)
    identified = sc["load_snapshot"](root)
    registry = tomllib.loads((root / sc["REGISTRY"]).read_text(encoding="utf-8"))
    decoder = sc["parse_decoder"]((root / sc["DECODER"]).read_text(encoding="utf-8"))
    generated = sc["parse_generated"]((root / sc["GENERATED"]).read_text(encoding="utf-8"))
    decoder_forms = set(decoder) | set(generated)
    declared, rules = sc["read_declarations"](registry, identified)
    status = tomllib.loads((root / sc["STATUS"]).read_text(encoding="utf-8"))
    _, encoded, lowered, reference, metal = _evidence_sets(sc, status, identified)
    return {
        "sc": sc,
        "identified": identified,
        "registry": registry,
        "rules": rules,
        "declared": declared,
        "decoder": decoder_forms,
        "encoded": encoded,
        "lowered": lowered,
        "reference": reference,
        "metal": metal,
        "status": status,
    }


def _evidence_sets(sc, status, identified):
    encoded, lowered, reference, metal = set(), set(), set(), set()
    seen = set()
    for row in status["instruction"]:
        key = sc["status_form"](row, identified)
        if key in seen:
            continue
        seen.add(key)
        if row.get("llvm") == "three-way passed":
            encoded.add(key)
        lowering = row.get("lowering", "").strip().lower()
        if lowering.startswith(("msl ", "resolved ", "verified ", "constant or ", "per-thread ")):
            lowered.add(key)
        decodes = row.get("decoded") is True
        if decodes and row.get("reference") == "implemented" and row.get("cpu") == "fixture verified":
            reference.add(key)
        if decodes and row.get("gpu") in {"fixture verified", "fixture verified on Apple M5"}:
            metal.add(key)
    return seen, encoded, lowered, reference, metal


def summarize_inventory(data: dict[str, Any]) -> Inventory:
    identified = data["identified"]
    declared = data["declared"]
    decoder = data["decoder"]
    reference = data["reference"]
    metal = data["metal"]
    unsupported_keys = set(identified) - set(declared)
    by_family: dict[str, dict[str, int]] = {}
    for family, name in identified:
        bucket = by_family.setdefault(family, {
            "identified": 0, "declared": 0, "decoder": 0, "unsupported": 0,
            "reference": 0, "metal": 0,
        })
        bucket["identified"] += 1
        key = (family, name)
        if key in declared:
            bucket["declared"] += 1
        if key in decoder:
            bucket["decoder"] += 1
        if key in unsupported_keys:
            bucket["unsupported"] += 1
        if key in reference:
            bucket["reference"] += 1
        if key in metal:
            bucket["metal"] += 1
    return Inventory(
        identified=len(identified),
        declared=len(declared),
        decoder_supported=len(decoder),
        reference_verified=len(reference),
        metal_verified=len(metal),
        unsupported=len(unsupported_keys),
        by_family=dict(sorted(by_family.items())),
    )


def _declared_ops(registry: dict[str, Any]) -> set[str]:
    return {form.get("operation") for form in registry.get("form", []) if form.get("operation")}


def _declared_rules(registry: dict[str, Any]) -> set[str]:
    return set(registry.get("operand_rules", {}))


def classify_unsupported(data: dict[str, Any], *, limit: int | None = None) -> list[Candidate]:
    """Build candidates for identified forms that are not registry-declared.

    Does not infer behaviour from the mnemonic. Width/type/layout are filled
    only when already present on related declared forms in the same family, or
    left null.
    """
    identified = data["identified"]
    declared = data["declared"]
    registry = data["registry"]
    rules = data["rules"]
    ops = _declared_ops(registry)
    rule_names = _declared_rules(registry)
    family_rules = defaultdict(list)
    family_widths = defaultdict(set)
    family_types = defaultdict(set)
    for form in registry.get("form", []):
        family_rules[form["family"]].append(form.get("operands"))
        if "width" in form:
            family_widths[form["family"]].add(form["width"])
        if "signedness" in form:
            family_types[form["family"]].add(form["signedness"])

    candidates: list[Candidate] = []
    unsupported = sorted(set(identified) - set(declared))
    if limit is not None:
        unsupported = unsupported[:limit]
    for family, name in unsupported:
        opcode = identified[(family, name)]
        reusable = sorted(op for op in ops if op)  # available primitives, not guessed mapping
        missing_rules = []
        # If the family has no operand rule yet, record that gap.
        family_rule_names = [n for n, (fam, _) in rules.items() if fam == family]
        if not family_rule_names:
            missing_rules.append(f"no operand_rules for family {family}")
        missing_effects = []
        # Architectural effects are unknown until specified; flag absence.
        missing_effects.append("scc/vcc/exec effects unspecified until registry declaration")
        widths = sorted(family_widths.get(family, []))
        types = sorted(family_types.get(family, []))
        layout = family_rule_names[0] if len(family_rule_names) == 1 else None
        batch = _batch_for(family, family_rule_names, ops)
        candidates.append(Candidate(
            family=family,
            name=name,
            opcode=opcode,
            width=widths[0] if len(widths) == 1 else None,
            numeric_type=types[0] if len(types) == 1 else None,
            operand_layout=layout,
            reusable_primitives=reusable[:20],
            missing_operand_rules=missing_rules,
            missing_effects=missing_effects,
            spec_refs=[{"document": "xml", "note": "identity from pinned AMD snapshot only"}],
            priority_batch=batch,
            notes=[
                "behaviour must come from the public AMD guide, never the mnemonic",
                "candidate is not implemented",
            ],
        ))
    return candidates


def _batch_for(family: str, family_rule_names: list[str], ops: set[str]) -> str:
    if family_rule_names and ops:
        return f"batch-{family.lower()}-reuse-primitives"
    if family_rule_names:
        return f"batch-{family.lower()}-needs-primitives"
    return f"batch-{family.lower()}-needs-operand-rules"


def plan_batches(candidates: list[Candidate]) -> dict[str, list[dict[str, Any]]]:
    batches: dict[str, list[dict[str, Any]]] = defaultdict(list)
    for cand in candidates:
        batches[cand.priority_batch].append({
            "family": cand.family,
            "name": cand.name,
            "opcode": cand.opcode,
            "missing_operand_rules": cand.missing_operand_rules,
            "reusable_primitives": cand.reusable_primitives[:5],
        })
    return dict(sorted(batches.items()))


def discrepancies(data: dict[str, Any]) -> list[dict[str, str]]:
    """Registry vs decoder vs verification evidence mismatches."""
    issues: list[dict[str, str]] = []
    declared = data["declared"]
    decoder = data["decoder"]
    reference = data["reference"]
    metal = data["metal"]
    for key in sorted(declared - decoder):
        family, name = key
        issues.append({
            "kind": "declared-not-decoded",
            "family": family,
            "name": name,
            "detail": "registry declaration is not present in decoder SEMANTIC_FORMS/FORM_IDS",
        })
    for key in sorted(decoder - declared):
        family, name = key
        issues.append({
            "kind": "decoded-not-declared",
            "family": family,
            "name": name,
            "detail": "decoder lists a form that is not in the semantic registry",
        })
    for key in sorted(reference - declared):
        family, name = key
        issues.append({
            "kind": "reference-without-declaration",
            "family": family,
            "name": name,
            "detail": "status marks reference verification without a registry declaration",
        })
    for key in sorted(metal - reference):
        family, name = key
        issues.append({
            "kind": "metal-without-reference",
            "family": family,
            "name": name,
            "detail": "Metal verification without matching reference evidence",
        })
    return issues


def coverage_diff(before: Inventory, after: Inventory) -> dict[str, Any]:
    return {
        "identified_delta": after.identified - before.identified,
        "declared_delta": after.declared - before.declared,
        "decoder_delta": after.decoder_supported - before.decoder_supported,
        "unsupported_delta": after.unsupported - before.unsupported,
        "reference_delta": after.reference_verified - before.reference_verified,
        "metal_delta": after.metal_verified - before.metal_verified,
        "before": asdict(before),
        "after": asdict(after),
    }


def dumps_candidates(candidates: list[Candidate]) -> str:
    payload = {
        "note": "Candidates are planning aids only; they are not implemented forms.",
        "count": len(candidates),
        "candidates": [asdict(c) for c in candidates],
    }
    return json.dumps(payload, indent=2, sort_keys=True) + "\n"


def dumps_inventory(inv: Inventory) -> str:
    return json.dumps(asdict(inv), indent=2, sort_keys=True) + "\n"
