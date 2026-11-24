# SPDX-License-Identifier: GPL-3.0-or-later
"""Application bundle structure and codesign diagnostic helpers."""
from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path
from typing import Iterable


REQUIRED_APP_ENTRIES = (
    "Contents/Info.plist",
    "Contents/MacOS",
)


@dataclass(frozen=True)
class BundleCheck:
    root: str
    ok: bool
    problems: tuple[str, ...]
    present: tuple[str, ...]


@dataclass(frozen=True)
class CodesignDiagnosis:
    path: str
    ok: bool
    problems: tuple[str, ...]
    notes: tuple[str, ...]


def check_app_bundle(root: Path, entries: Iterable[str] | None = None) -> BundleCheck:
    required = tuple(entries) if entries is not None else REQUIRED_APP_ENTRIES
    present: list[str] = []
    problems: list[str] = []
    if not root.exists():
        return BundleCheck(root=str(root), ok=False, problems=(f"missing bundle root: {root}",), present=())
    if not root.is_dir():
        return BundleCheck(root=str(root), ok=False, problems=(f"bundle root is not a directory: {root}",), present=())
    for rel in required:
        path = root / rel
        if path.exists():
            present.append(rel)
        else:
            problems.append(f"missing {rel}")
    return BundleCheck(root=str(root), ok=not problems, problems=tuple(problems), present=tuple(present))


def diagnose_codesign_output(path: str, codesign_stderr_or_stdout: str) -> CodesignDiagnosis:
    text = codesign_stderr_or_stdout.strip()
    notes = tuple(line.strip() for line in text.splitlines() if line.strip())
    problems: list[str] = []
    lower = text.casefold()
    if "code object is not signed at all" in lower:
        problems.append("unsigned")
    if "cssmerr_tp_cert_expired" in lower or "expired" in lower:
        problems.append("certificate expired or reported expired")
    if "errsec" in lower or "failed" in lower:
        if not problems:
            problems.append("codesign reported failure")
    ok = not problems
    if not text:
        problems.append("empty codesign output")
        ok = False
    return CodesignDiagnosis(path=path, ok=ok, problems=tuple(problems), notes=notes)
