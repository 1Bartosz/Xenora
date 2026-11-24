# SPDX-License-Identifier: GPL-3.0-or-later
"""ARM64 Mach-O architecture and linkage inspection helpers."""
from __future__ import annotations

import re
from dataclasses import dataclass
from typing import Iterable, Sequence

FORBIDDEN_LINKAGE = ("vulkan", "moltenvk", "libvulkan")


@dataclass(frozen=True)
class ArchCheck:
    path: str
    archs: tuple[str, ...]
    ok: bool
    problems: tuple[str, ...]


@dataclass(frozen=True)
class LinkageCheck:
    path: str
    lines: tuple[str, ...]
    ok: bool
    problems: tuple[str, ...]


def parse_lipo_archs(text: str) -> tuple[str, ...]:
    parts = [p for p in text.strip().split() if p]
    return tuple(parts)


def check_archs(path: str, arch_text: str, *, require: Sequence[str] = ("arm64",)) -> ArchCheck:
    archs = parse_lipo_archs(arch_text)
    problems: list[str] = []
    required = tuple(require)
    if archs != required:
        problems.append(f"expected archs {required}, got {archs}")
    return ArchCheck(path=path, archs=archs, ok=not problems, problems=tuple(problems))


def check_otool_linkage(path: str, otool_text: str) -> LinkageCheck:
    lines = tuple(line.rstrip() for line in otool_text.splitlines() if line.strip())
    problems: list[str] = []
    joined = "\n".join(lines).casefold()
    for name in FORBIDDEN_LINKAGE:
        if name in joined:
            problems.append(f"forbidden linkage mention: {name}")
    return LinkageCheck(path=path, lines=lines, ok=not problems, problems=tuple(problems))


_DEPLOY_RE = re.compile(r"minos|sdk|version.min", re.IGNORECASE)


def deployment_target_tokens(otool_header_text: str) -> tuple[str, ...]:
    tokens: list[str] = []
    for line in otool_header_text.splitlines():
        if _DEPLOY_RE.search(line):
            tokens.append(line.strip())
    return tuple(tokens)


def consistent_deployment_targets(texts: Iterable[str]) -> tuple[bool, tuple[str, ...]]:
    sets = [frozenset(deployment_target_tokens(t)) for t in texts]
    if not sets:
        return True, ()
    first = sets[0]
    ok = all(s == first for s in sets[1:])
    return ok, tuple(sorted(first))
