# SPDX-License-Identifier: GPL-3.0-or-later
"""Advisory pull-request validation helpers.

Checks changed paths against an approved scope, DCO trailers, native arch
hints, changelog presence and accidental build-artifact inclusion. Advisory
only: callers decide whether to fail a gate.
"""
from __future__ import annotations

import re
from dataclasses import dataclass, field
from pathlib import PurePosixPath
from typing import Iterable, Sequence

DCO_RE = re.compile(r"^Signed-off-by:\s+\S.+\s+<\S+@\S+>\s*$", re.MULTILINE)
FULL_SHA_RE = re.compile(r"^[0-9a-f]{40}$")
ARTIFACT_SUFFIXES = (
    ".o", ".a", ".dylib", ".so", ".pyc", ".rlib", ".rmeta",
)
ARTIFACT_NAMES = {"__pycache__", "target", ".DS_Store"}


@dataclass
class Finding:
    code: str
    severity: str  # advisory | blocking-candidate
    message: str


@dataclass
class ValidationReport:
    findings: list[Finding] = field(default_factory=list)

    @property
    def ok(self) -> bool:
        return not any(f.severity == "blocking-candidate" for f in self.findings)

    def add(self, code: str, message: str, *, severity: str = "advisory") -> None:
        self.findings.append(Finding(code=code, severity=severity, message=message))


def path_in_scope(path: str, patterns: Sequence[str]) -> bool:
    posix = PurePosixPath(path)
    for pattern in patterns:
        pat = pattern.rstrip()
        if pat.endswith("/**"):
            root = pat[:-3]
            if path == root or path.startswith(root.rstrip("/") + "/"):
                return True
        elif pat.endswith("/"):
            if path.startswith(pat) or path == pat.rstrip("/"):
                return True
        elif "*" in pat or "?" in pat:
            if posix.match(pat):
                return True
        elif path == pat:
            return True
    return False


def check_scope(changed_files: Iterable[str], allowed: Sequence[str]) -> ValidationReport:
    report = ValidationReport()
    for path in changed_files:
        if not path_in_scope(path, allowed):
            report.add(
                "scope",
                f"changed file outside approved scope: {path}",
                severity="blocking-candidate",
            )
    return report


def check_dco(commit_messages: Iterable[str]) -> ValidationReport:
    report = ValidationReport()
    for i, message in enumerate(commit_messages, start=1):
        if not DCO_RE.search(message or ""):
            report.add(
                "dco",
                f"commit {i} is missing a Signed-off-by trailer",
                severity="blocking-candidate",
            )
    return report


def check_artifacts(changed_files: Iterable[str]) -> ValidationReport:
    report = ValidationReport()
    for path in changed_files:
        name = PurePosixPath(path).name
        if name in ARTIFACT_NAMES or any(part in ARTIFACT_NAMES for part in PurePosixPath(path).parts):
            report.add("artifact", f"build artifact path included: {path}")
            continue
        if name.endswith(ARTIFACT_SUFFIXES):
            report.add("artifact", f"build artifact suffix included: {path}")
    return report


def check_changelog_required(changed_files: Iterable[str], *, require: bool = True) -> ValidationReport:
    report = ValidationReport()
    files = list(changed_files)
    meaningful = [
        f for f in files
        if not f.startswith("docs/") and f != "CHANGELOG.md" and not f.startswith("tests/")
    ]
    if require and meaningful and "CHANGELOG.md" not in files:
        report.add(
            "changelog",
            "meaningful non-docs code changes without CHANGELOG.md",
            severity="advisory",
        )
    return report


def check_report_commit_identity(report_sha: str, head_sha: str) -> ValidationReport:
    report = ValidationReport()
    if not FULL_SHA_RE.fullmatch(report_sha or ""):
        report.add("evidence-sha", "test report commit is not a full 40-character SHA",
                   severity="blocking-candidate")
        return report
    if report_sha != head_sha:
        report.add(
            "evidence-sha",
            f"test report SHA {report_sha} does not match head {head_sha}",
            severity="blocking-candidate",
        )
    return report


def check_native_arch_strings(file_output_lines: Iterable[str]) -> ValidationReport:
    """Advisory scan of `file`/`lipo` style lines for non-arm64 host binaries."""
    report = ValidationReport()
    for line in file_output_lines:
        lower = line.lower()
        if "mach-o" not in lower:
            continue
        if "arm64" in lower or "aarch64" in lower:
            continue
        if "x86_64" in lower or "i386" in lower:
            report.add(
                "arch",
                f"non-ARM64 Mach-O mention: {line.strip()}",
                severity="blocking-candidate",
            )
    return report


def merge_reports(*reports: ValidationReport) -> ValidationReport:
    out = ValidationReport()
    for report in reports:
        out.findings.extend(report.findings)
    return out
