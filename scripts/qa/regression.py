# SPDX-License-Identifier: GPL-3.0-or-later
"""Historical orchestration report comparison."""
from __future__ import annotations

import json
from dataclasses import dataclass
from typing import Any


@dataclass(frozen=True)
class SuiteDelta:
    name: str
    previous_status: str | None
    current_status: str | None
    kind: str


@dataclass(frozen=True)
class RegressionDiff:
    newly_failed: tuple[SuiteDelta, ...]
    newly_passed: tuple[SuiteDelta, ...]
    still_failed: tuple[SuiteDelta, ...]
    missing: tuple[SuiteDelta, ...]


def load_report(text: str) -> dict[str, Any]:
    return json.loads(text)


def _status_map(report: dict[str, Any]) -> dict[str, tuple[str, str]]:
    out: dict[str, tuple[str, str]] = {}
    for suite in report.get("suites", []):
        out[suite["name"]] = (suite.get("status", "unknown"), suite.get("kind", "other"))
    return out


def compare_reports(previous: dict[str, Any], current: dict[str, Any]) -> RegressionDiff:
    prev = _status_map(previous)
    cur = _status_map(current)
    newly_failed: list[SuiteDelta] = []
    newly_passed: list[SuiteDelta] = []
    still_failed: list[SuiteDelta] = []
    missing: list[SuiteDelta] = []
    for name, (status, kind) in sorted(cur.items()):
        before = prev.get(name)
        before_status = before[0] if before else None
        if status == "failed" and before_status != "failed":
            newly_failed.append(SuiteDelta(name, before_status, status, kind))
        elif status == "passed" and before_status == "failed":
            newly_passed.append(SuiteDelta(name, before_status, status, kind))
        elif status == "failed" and before_status == "failed":
            still_failed.append(SuiteDelta(name, before_status, status, kind))
    for name, (status, kind) in sorted(prev.items()):
        if name not in cur:
            missing.append(SuiteDelta(name, status, None, kind))
    return RegressionDiff(
        newly_failed=tuple(newly_failed),
        newly_passed=tuple(newly_passed),
        still_failed=tuple(still_failed),
        missing=tuple(missing),
    )


def has_new_regressions(diff: RegressionDiff) -> bool:
    return bool(diff.newly_failed or diff.missing)
