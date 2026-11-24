# SPDX-License-Identifier: GPL-3.0-or-later
"""Compatibility gap analysis over read-only dependency reports.

Consumes JSON describing imports required by authorized workloads. Never
executes input binaries. Groups missing symbols by reusable subsystem.
"""
from __future__ import annotations

import json
from collections import defaultdict
from dataclasses import asdict, dataclass, field
from pathlib import Path
from typing import Any, Iterable


# Heuristic subsystem buckets for missing Win64/CRT symbols (planning only).
SUBSYSTEM_RULES: tuple[tuple[str, tuple[str, ...]], ...] = (
    ("memory", ("VirtualAlloc", "VirtualFree", "HeapAlloc", "HeapFree", "LocalAlloc", "GlobalAlloc")),
    ("threading", ("CreateThread", "OpenThread", "WaitFor", "Sleep", "Tls", "CriticalSection")),
    ("process", ("GetCurrentProcess", "GetProcess", "TerminateProcess", "ExitProcess", "CreateProcess")),
    ("error", ("GetLastError", "SetLastError", "FormatMessage")),
    ("time", ("GetSystemTime", "QueryPerformance", "GetTickCount", "Sleep")),
    ("crt-math", ("sqrt", "fabs", "sin", "cos", "pow", "exp", "log", "floor", "ceil")),
    ("crt-string", ("strlen", "strcmp", "strcpy", "memcpy", "memmove", "memset", "sprintf", "snprintf")),
    ("file-io", ("CreateFile", "ReadFile", "WriteFile", "CloseHandle", "GetFileSize")),
    ("sync", ("WaitForSingleObject", "WaitForMultipleObjects", "CreateEvent", "CreateMutex", "ReleaseMutex")),
)


@dataclass
class GapItem:
    module: str
    symbol: str
    subsystem: str
    status: str  # missing | partial | present
    note: str = ""


@dataclass
class GapReport:
    workload: str
    total_imports: int
    missing: list[dict[str, Any]] = field(default_factory=list)
    partial: list[dict[str, Any]] = field(default_factory=list)
    present: list[dict[str, Any]] = field(default_factory=list)
    by_subsystem: dict[str, list[str]] = field(default_factory=dict)
    shared_primitives: list[str] = field(default_factory=list)
    notes: list[str] = field(default_factory=list)

    def to_dict(self) -> dict[str, Any]:
        return asdict(self)


def classify_subsystem(symbol: str) -> str:
    for name, needles in SUBSYSTEM_RULES:
        for needle in needles:
            if needle.lower() in symbol.lower():
                return name
    return "other"


def _normalize_imports(payload: dict[str, Any]) -> list[tuple[str, str]]:
    """Accept {workload, imports:[{module,symbol}| "mod!sym"]} or flat list."""
    items = payload.get("imports", payload if isinstance(payload, list) else [])
    out: list[tuple[str, str]] = []
    for item in items:
        if isinstance(item, str):
            if "!" in item:
                mod, sym = item.split("!", 1)
            else:
                mod, sym = "unknown", item
            out.append((mod, sym))
        elif isinstance(item, dict):
            mod = str(item.get("module") or item.get("dll") or "unknown")
            sym = str(item.get("symbol") or item.get("name") or "")
            if sym:
                out.append((mod, sym))
    return out


def analyze_gaps(
    dependency_report: dict[str, Any],
    *,
    implemented: Iterable[str],
    partial: Iterable[str],
) -> GapReport:
    """Compare required imports to runtime inventory keys (`dll!symbol`)."""
    impl = {k.lower() for k in implemented}
    part = {k.lower() for k in partial}
    workload = str(dependency_report.get("workload", "unspecified"))
    imports = _normalize_imports(dependency_report)
    missing: list[GapItem] = []
    partial_hits: list[GapItem] = []
    present: list[GapItem] = []
    for module, symbol in imports:
        key = f"{module}!{symbol}".lower()
        # Also try basename match on symbol alone against inventory keys.
        symbol_keys = {k for k in impl | part if k.endswith("!" + symbol.lower())}
        if key in impl or any(k in impl for k in symbol_keys):
            present.append(GapItem(module, symbol, classify_subsystem(symbol), "present"))
        elif key in part or any(k in part for k in symbol_keys):
            partial_hits.append(GapItem(module, symbol, classify_subsystem(symbol), "partial",
                                        "registry status is partial"))
        else:
            missing.append(GapItem(module, symbol, classify_subsystem(symbol), "missing"))

    by_sub: dict[str, list[str]] = defaultdict(list)
    for item in missing + partial_hits:
        by_sub[item.subsystem].append(f"{item.module}!{item.symbol}")
    # Shared primitives: subsystems with more than one missing/partial entry.
    shared = sorted(sub for sub, items in by_sub.items() if len(items) > 1 and sub != "other")
    return GapReport(
        workload=workload,
        total_imports=len(imports),
        missing=[asdict(i) for i in missing],
        partial=[asdict(i) for i in partial_hits],
        present=[asdict(i) for i in present],
        by_subsystem={k: sorted(v) for k, v in sorted(by_sub.items())},
        shared_primitives=shared,
        notes=[
            "Read-only analysis of a dependency JSON report; binaries are not executed.",
            "Subsystem grouping is advisory for prioritising shared primitives.",
            "Does not claim Xbox or commercial workload compatibility.",
        ],
    )


def load_dependency_report(path: Path) -> dict[str, Any]:
    return json.loads(path.read_text(encoding="utf-8"))


def compact_gaps(report: GapReport) -> str:
    return (
        f"workload={report.workload} imports={report.total_imports} "
        f"missing={len(report.missing)} partial={len(report.partial)} "
        f"present={len(report.present)} shared_subsystems={len(report.shared_primitives)}\n"
    )


def dumps_gaps(report: GapReport) -> str:
    return json.dumps(report.to_dict(), indent=2, sort_keys=True) + "\n"
