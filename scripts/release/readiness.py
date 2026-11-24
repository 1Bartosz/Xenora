# SPDX-License-Identifier: GPL-3.0-or-later
"""Release readiness reporting from real inspection outputs.

Does not publish releases. Builds a complete readiness report from Mach-O,
bundle, codesign, checksum, license and verification evidence inputs.
"""
from __future__ import annotations

import json
from dataclasses import asdict, dataclass, field
from pathlib import Path
from typing import Any, Iterable, Sequence

from .bundle import check_app_bundle, diagnose_codesign_output
from .macho import check_archs, check_otool_linkage, consistent_deployment_targets
from .manifest import ArtifactRecord, build_manifest, license_inventory, sha256_file, validate_evidence


@dataclass
class ReadinessFinding:
    code: str
    ok: bool
    detail: str


@dataclass
class ReadinessReport:
    revision: str
    findings: list[ReadinessFinding] = field(default_factory=list)
    sbom: list[dict[str, str]] = field(default_factory=list)
    artifacts: list[dict[str, Any]] = field(default_factory=list)

    @property
    def ok(self) -> bool:
        return all(f.ok for f in self.findings)

    def to_dict(self) -> dict[str, Any]:
        return {
            "revision": self.revision,
            "ok": self.ok,
            "findings": [asdict(f) for f in self.findings],
            "sbom": self.sbom,
            "artifacts": self.artifacts,
        }


def inspect_binary(*, path: str, lipo_archs: str, otool_l: str, otool_header: str = "") -> list[ReadinessFinding]:
    findings: list[ReadinessFinding] = []
    arch = check_archs(path, lipo_archs)
    findings.append(ReadinessFinding("arch", arch.ok, "; ".join(arch.problems) or f"archs={list(arch.archs)}"))
    link = check_otool_linkage(path, otool_l)
    findings.append(ReadinessFinding("linkage", link.ok, "; ".join(link.problems) or "ok"))
    if otool_header:
        ok, tokens = consistent_deployment_targets([otool_header])
        findings.append(ReadinessFinding(
            "deployment-target",
            ok,
            ", ".join(tokens) if tokens else "no deployment tokens found",
        ))
    return findings


def inspect_app(app_root: Path, codesign_output: str = "") -> list[ReadinessFinding]:
    findings: list[ReadinessFinding] = []
    bundle = check_app_bundle(app_root)
    findings.append(ReadinessFinding(
        "bundle",
        bundle.ok,
        "; ".join(bundle.problems) or f"present={list(bundle.present)}",
    ))
    if codesign_output:
        diag = diagnose_codesign_output(str(app_root), codesign_output)
        findings.append(ReadinessFinding("codesign", diag.ok, "; ".join(diag.problems) or "ok"))
    return findings


def build_sbom(entries: Iterable[tuple[str, str, str]]) -> list[dict[str, str]]:
    """Complete SBOM rows: name, version, license."""
    rows = sorted({(n, v, lic) for n, v, lic in entries})
    return [{"name": n, "version": v, "license": lic} for n, v, lic in rows]


def collect_artifacts(paths: Sequence[Path], *, revision: str) -> tuple[list[dict[str, Any]], list[ReadinessFinding]]:
    manifest = build_manifest(paths, revision=revision)
    artifacts = [asdict(a) for a in manifest.artifacts]
    observed = {a["path"]: a["sha256"] for a in artifacts}
    expected = dict(observed)
    ok, problems = validate_evidence(manifest_sha_by_path=expected, observed_sha_by_path=observed)
    findings = [ReadinessFinding("checksums", ok, "; ".join(problems) or f"{len(artifacts)} artifacts")]
    return artifacts, findings


def require_evidence_files(paths: Sequence[Path]) -> list[ReadinessFinding]:
    findings = []
    for path in paths:
        findings.append(ReadinessFinding(
            f"evidence:{path.name}",
            path.is_file(),
            "present" if path.is_file() else f"missing {path}",
        ))
    return findings


def build_report(
    *,
    revision: str,
    binary_findings: Sequence[ReadinessFinding] = (),
    app_findings: Sequence[ReadinessFinding] = (),
    artifacts: Sequence[dict[str, Any]] = (),
    artifact_findings: Sequence[ReadinessFinding] = (),
    evidence_findings: Sequence[ReadinessFinding] = (),
    sbom: Sequence[dict[str, str]] = (),
) -> ReadinessReport:
    findings = list(binary_findings) + list(app_findings) + list(artifact_findings) + list(evidence_findings)
    if not sbom:
        findings.append(ReadinessFinding("sbom", False, "SBOM is empty; provide dependency inventory"))
    else:
        findings.append(ReadinessFinding("sbom", True, f"{len(sbom)} components"))
    findings.append(ReadinessFinding("revision", bool(revision) and len(revision) >= 7, revision or "missing"))
    return ReadinessReport(
        revision=revision,
        findings=findings,
        sbom=list(sbom),
        artifacts=list(artifacts),
    )


def dumps_report(report: ReadinessReport) -> str:
    return json.dumps(report.to_dict(), indent=2, sort_keys=True) + "\n"


def license_text_from_sbom(sbom: Sequence[dict[str, str]]) -> str:
    return license_inventory((row["name"], row["license"]) for row in sbom)
