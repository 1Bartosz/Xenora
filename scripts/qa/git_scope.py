# SPDX-License-Identifier: GPL-3.0-or-later
"""Shared-file collision helpers that account for renames and deletions."""
from __future__ import annotations

from dataclasses import dataclass
from typing import Iterable, Sequence


@dataclass(frozen=True)
class PathChange:
    status: str  # A | M | D | R
    path: str
    old_path: str | None = None


def parse_name_status(lines: Iterable[str]) -> list[PathChange]:
    """Parse `git diff --name-status` style lines, including renames."""
    out: list[PathChange] = []
    for raw in lines:
        line = raw.strip("\n")
        if not line:
            continue
        parts = line.split("\t")
        status = parts[0]
        code = status[0]
        if code == "R" and len(parts) >= 3:
            out.append(PathChange("R", parts[2], old_path=parts[1]))
        elif code == "D" and len(parts) >= 2:
            out.append(PathChange("D", parts[1]))
        elif len(parts) >= 2:
            out.append(PathChange(code, parts[1]))
    return out


def effective_paths(changes: Sequence[PathChange]) -> set[str]:
    """Paths that still collide for ownership: added/modified/renamed targets.

    Deleted paths are excluded from forward collision checks but retained via
    `deleted_paths` for reports.
    """
    paths: set[str] = set()
    for change in changes:
        if change.status == "D":
            continue
        paths.add(change.path)
        if change.status == "R" and change.old_path:
            # Renames collide on both old and new until the claim scope updates.
            paths.add(change.old_path)
    return paths


def deleted_paths(changes: Sequence[PathChange]) -> set[str]:
    deleted = {c.path for c in changes if c.status == "D"}
    deleted |= {c.old_path for c in changes if c.status == "R" and c.old_path}
    return {p for p in deleted if p}


def directory_overlap(a: str, b: str) -> bool:
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
        a = a.rstrip("/")
        return b == a or b.startswith(a + "/")
    if b.endswith("/"):
        b = b.rstrip("/")
        return a == b or a.startswith(b + "/")
    return a.startswith(b + "/") or b.startswith(a + "/")


def colliding_scopes(changed: Iterable[str], claimed_patterns: Sequence[str]) -> list[tuple[str, str]]:
    hits: list[tuple[str, str]] = []
    for path in changed:
        for pattern in claimed_patterns:
            if directory_overlap(path, pattern) or directory_overlap(pattern, path):
                hits.append((path, pattern))
    return hits
