# SPDX-License-Identifier: GPL-3.0-or-later
"""Historical RDNA2 coverage snapshots for revision-to-revision reports."""
from __future__ import annotations

import json
from dataclasses import asdict
from pathlib import Path
from typing import Any

from .rdna2_plan import Inventory, coverage_diff, dumps_inventory, load_inventories, summarize_inventory


def snapshot_inventory(root: Path) -> Inventory:
    return summarize_inventory(load_inventories(root))


def write_snapshot(path: Path, inventory: Inventory, *, revision: str) -> None:
    payload = asdict(inventory)
    payload["revision"] = revision
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def read_snapshot(path: Path) -> tuple[str, Inventory]:
    data = json.loads(path.read_text(encoding="utf-8"))
    revision = str(data.pop("revision", "unknown"))
    inv = Inventory(
        identified=int(data["identified"]),
        declared=int(data["declared"]),
        decoder_supported=int(data["decoder_supported"]),
        reference_verified=int(data["reference_verified"]),
        metal_verified=int(data["metal_verified"]),
        unsupported=int(data["unsupported"]),
        by_family=dict(data.get("by_family", {})),
    )
    return revision, inv


def historical_report(before_path: Path, after_path: Path) -> dict[str, Any]:
    before_rev, before = read_snapshot(before_path)
    after_rev, after = read_snapshot(after_path)
    diff = coverage_diff(before, after)
    diff["before_revision"] = before_rev
    diff["after_revision"] = after_rev
    return diff
