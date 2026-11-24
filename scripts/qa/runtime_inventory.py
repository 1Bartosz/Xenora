# SPDX-License-Identifier: GPL-3.0-or-later
"""Typed runtime service inventory from crates/loader/runtime/services.txt.

Separates canonical services from API-set aliases. Does not execute guests or
modify runtime crates.
"""
from __future__ import annotations

import json
import re
from collections import defaultdict
from dataclasses import asdict, dataclass, field
from pathlib import Path
from typing import Any


SERVICES_REL = Path("crates/loader/runtime/services.txt")
STATUS_VALUES = frozenset({"implemented", "partial", "stub"})


@dataclass
class ServiceRecord:
    library: str
    symbol: str
    signature: str
    handler: str
    status: str
    aliases: list[str] = field(default_factory=list)
    fails: str = ""
    ref: str = ""
    tests: list[str] = field(default_factory=list)

    @property
    def key(self) -> str:
        return f"{self.library}!{self.symbol}"


@dataclass
class RuntimeInventory:
    service_count: int
    by_status: dict[str, int]
    implemented: list[str]
    partial: list[str]
    stub: list[str]
    independently_verified: list[str]
    aliases: list[dict[str, str]]
    alias_count: int
    libraries: dict[str, int]
    services: list[dict[str, Any]]
    notes: list[str]

    def to_dict(self) -> dict[str, Any]:
        return asdict(self)


_SERVICE_HEAD = re.compile(
    r"^service\s+(\S+)\s+(\S+)\((.*)\)\s*->\s*(\S+)\s*$"
)


def parse_services_text(text: str) -> list[ServiceRecord]:
    services: list[ServiceRecord] = []
    current: ServiceRecord | None = None
    for raw in text.splitlines():
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        if line.startswith("service "):
            match = _SERVICE_HEAD.match(line)
            if not match:
                raise ValueError(f"bad service header: {line}")
            library, symbol, params, result = match.groups()
            current = ServiceRecord(
                library=library,
                symbol=symbol,
                signature=f"({params}) -> {result}",
                handler="",
                status="",
            )
            services.append(current)
            continue
        if current is None:
            raise ValueError(f"field outside service: {line}")
        if " " not in line:
            raise ValueError(f"missing field value: {line}")
        key, value = line.split(None, 1)
        if key == "handler":
            current.handler = value
        elif key == "status":
            if value not in STATUS_VALUES:
                raise ValueError(f"unknown status {value!r}")
            current.status = value
        elif key == "alias":
            current.aliases.append(value)
        elif key == "fails":
            current.fails = value
        elif key == "ref":
            current.ref = value
        elif key == "test":
            current.tests.append(value)
        else:
            raise ValueError(f"unknown field {key!r}")
    for svc in services:
        if not svc.handler or not svc.status:
            raise ValueError(f"incomplete service {svc.key}")
    return services


def build_runtime_inventory(root: Path) -> RuntimeInventory:
    path = root / SERVICES_REL
    services = parse_services_text(path.read_text(encoding="utf-8"))
    by_status: dict[str, int] = defaultdict(int)
    libraries: dict[str, int] = defaultdict(int)
    aliases: list[dict[str, str]] = []
    for svc in services:
        by_status[svc.status] += 1
        libraries[svc.library] += 1
        for alias in svc.aliases:
            aliases.append({
                "alias": alias,
                "canonical": svc.key,
                "symbol": svc.symbol,
            })
    # Independently verified: registry documents zero acceptance unless a future
    # field appears; do not invent. Track explicit empty list.
    independently_verified: list[str] = []
    notes = [
        "Status values are registry contracts for synthetic guests, not Xbox compatibility.",
        "API-set aliases are counted separately from canonical services.",
        "Independently verified remains empty until an acceptance process records it; not inferred.",
        "docs/runtime/services.md counts are generated; this tool re-parses services.txt.",
    ]
    return RuntimeInventory(
        service_count=len(services),
        by_status=dict(sorted(by_status.items())),
        implemented=sorted(s.key for s in services if s.status == "implemented"),
        partial=sorted(s.key for s in services if s.status == "partial"),
        stub=sorted(s.key for s in services if s.status == "stub"),
        independently_verified=independently_verified,
        aliases=sorted(aliases, key=lambda a: (a["alias"], a["canonical"])),
        alias_count=len(aliases),
        libraries=dict(sorted(libraries.items())),
        services=[asdict(s) for s in services],
        notes=notes,
    )


def compact_runtime(inv: RuntimeInventory) -> str:
    return (
        f"services={inv.service_count} "
        f"implemented={inv.by_status.get('implemented', 0)} "
        f"partial={inv.by_status.get('partial', 0)} "
        f"stub={inv.by_status.get('stub', 0)} "
        f"aliases={inv.alias_count} "
        f"verified={len(inv.independently_verified)}\n"
    )


def dumps_runtime(inv: RuntimeInventory, *, include_services: bool = True) -> str:
    data = inv.to_dict()
    if not include_services:
        data.pop("services", None)
    return json.dumps(data, indent=2, sort_keys=True) + "\n"
