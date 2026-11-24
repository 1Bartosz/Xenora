# SPDX-License-Identifier: GPL-3.0-or-later
"""Contributor progress summaries and engineering handoff reports."""
from __future__ import annotations

from dataclasses import dataclass
from typing import Iterable, Sequence


@dataclass(frozen=True)
class WorkItem:
    issue: int
    title: str
    status: str
    paths: tuple[str, ...]
    notes: str = ""


@dataclass(frozen=True)
class HandoffReport:
    author: str
    revision: str
    items: tuple[WorkItem, ...]
    limitations: tuple[str, ...]


def progress_summary(items: Iterable[WorkItem]) -> str:
    rows = list(items)
    lines = ["# Contributor progress", "", f"items: {len(rows)}", ""]
    for item in rows:
        paths = ", ".join(f"`{p}`" for p in item.paths) or "(none)"
        lines.append(f"- #{item.issue} [{item.status}] {item.title} — {paths}")
        if item.notes:
            lines.append(f"  - {item.notes}")
    lines.append("")
    return "\n".join(lines)


def handoff_markdown(report: HandoffReport) -> str:
    lines = [
        "# Engineering handoff",
        "",
        f"- author: `{report.author}`",
        f"- revision: `{report.revision}`",
        "",
        "## Work items",
        "",
    ]
    lines.append(progress_summary(report.items).split("\n", 2)[-1].lstrip("\n"))
    if not report.items:
        lines.append("(none)")
        lines.append("")
    lines.extend(["## Limitations", ""])
    if report.limitations:
        for lim in report.limitations:
            lines.append(f"- {lim}")
    else:
        lines.append("- (none recorded)")
    lines.append("")
    return "\n".join(lines)


def conflict_paths(items: Sequence[WorkItem]) -> tuple[tuple[int, int, str], ...]:
    """Return overlapping path pairs between distinct open items."""
    hits: list[tuple[int, int, str]] = []
    open_items = [i for i in items if i.status in {"claimed", "in-progress", "open"}]
    for i, a in enumerate(open_items):
        for b in open_items[i + 1 :]:
            for pa in a.paths:
                for pb in b.paths:
                    if paths_overlap(pa, pb):
                        hits.append((a.issue, b.issue, pa if len(pa) <= len(pb) else pb))
    return tuple(hits)


def paths_overlap(a: str, b: str) -> bool:
    a = a.rstrip("/")
    b = b.rstrip("/")
    if a == b:
        return True
    if a.endswith("/**"):
        root = a[:-3].rstrip("/")
        return b == root or b.startswith(root + "/")
    if b.endswith("/**"):
        root = b[:-3].rstrip("/")
        return a == root or a.startswith(root + "/")
    if a.endswith("/"):
        return b.startswith(a) or b == a.rstrip("/")
    if b.endswith("/"):
        return a.startswith(b) or a == b.rstrip("/")
    return a.startswith(b + "/") or b.startswith(a + "/")
