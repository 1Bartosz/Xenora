# SPDX-License-Identifier: GPL-3.0-or-later
"""Stale branch and divergent history diagnostics (read-only)."""
from __future__ import annotations

from dataclasses import asdict, dataclass
from typing import Iterable


@dataclass(frozen=True)
class BranchSync:
    branch: str
    upstream: str
    ahead: int
    behind: int
    diverged: bool
    stale: bool
    detail: str


def classify_sync(
    *,
    branch: str,
    upstream: str,
    ahead: int,
    behind: int,
    stale_behind_threshold: int = 1,
) -> BranchSync:
    diverged = ahead > 0 and behind > 0
    stale = behind >= stale_behind_threshold
    if diverged:
        detail = f"diverged: ahead={ahead} behind={behind}"
    elif stale:
        detail = f"behind upstream by {behind}"
    elif ahead > 0:
        detail = f"ahead of upstream by {ahead}"
    else:
        detail = "in sync"
    return BranchSync(
        branch=branch,
        upstream=upstream,
        ahead=ahead,
        behind=behind,
        diverged=diverged,
        stale=stale,
        detail=detail,
    )


def format_sync_reports(reports: Iterable[BranchSync]) -> str:
    lines = ["# Branch synchronization", ""]
    for report in reports:
        if report.diverged:
            mark = "DIVERGED"
        elif report.stale:
            mark = "STALE"
        else:
            mark = "ok"
        lines.append(
            f"- [{mark}] `{report.branch}` vs `{report.upstream}`: {report.detail}"
        )
    lines.append("")
    return "\n".join(lines)


def as_jsonable(report: BranchSync) -> dict:
    return asdict(report)
