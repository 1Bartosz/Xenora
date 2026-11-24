# SPDX-License-Identifier: GPL-3.0-or-later
"""Read-only claim conflict prediction for contributors.

Wraps the same path-overlap idea used by the agent queue without requiring a
network call. Useful before posting /claim or starting edits.
"""
from __future__ import annotations

from dataclasses import dataclass
from typing import Iterable, Sequence


@dataclass(frozen=True)
class ClaimSnapshot:
    number: int
    holder: str
    files: tuple[str, ...]
    claimed: bool


@dataclass(frozen=True)
class ConflictHit:
    candidate: int
    held_by: int
    holder: str
    pattern_a: str
    pattern_b: str


def _dir_prefix(pattern: str) -> str | None:
    if pattern.endswith("/**"):
        return pattern[:-3].rstrip("/")
    if pattern.endswith("/"):
        return pattern.rstrip("/")
    return None


def patterns_overlap(a: str, b: str) -> bool:
    if a == b:
        return True
    pa, pb = _dir_prefix(a), _dir_prefix(b)
    if pa is not None and (b == pa or b.startswith(pa + "/") or _dir_prefix(b) == pa):
        return True
    if pb is not None and (a == pb or a.startswith(pb + "/") or _dir_prefix(a) == pb):
        return True
    # prefix directory without slash: treat trailing path segment containment
    if a.endswith("/") and (b.startswith(a) or b == a.rstrip("/")):
        return True
    if b.endswith("/") and (a.startswith(b) or a == b.rstrip("/")):
        return True
    return False


def predict_conflicts(candidate_files: Sequence[str], held: Iterable[ClaimSnapshot]) -> tuple[ConflictHit, ...]:
    hits: list[ConflictHit] = []
    for claim in held:
        if not claim.claimed:
            continue
        for ca in candidate_files:
            for hb in claim.files:
                if patterns_overlap(ca, hb):
                    hits.append(ConflictHit(
                        candidate=-1,
                        held_by=claim.number,
                        holder=claim.holder,
                        pattern_a=ca,
                        pattern_b=hb,
                    ))
                    break
            else:
                continue
            break
    return tuple(hits)


def format_conflicts(hits: Sequence[ConflictHit]) -> str:
    if not hits:
        return "no file conflicts predicted\n"
    lines = ["predicted claim conflicts:", ""]
    for hit in hits:
        lines.append(
            f"- overlaps claimed #{hit.held_by} by {hit.holder}: `{hit.pattern_a}` vs `{hit.pattern_b}`"
        )
    lines.append("")
    return "\n".join(lines)
