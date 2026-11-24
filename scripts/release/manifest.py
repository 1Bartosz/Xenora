# SPDX-License-Identifier: GPL-3.0-or-later
"""Reproducible artifact manifests, checksums and simple SBOM stubs."""
from __future__ import annotations

import hashlib
import json
from dataclasses import asdict, dataclass
from pathlib import Path
from typing import Iterable


@dataclass(frozen=True)
class ArtifactRecord:
    path: str
    sha256: str
    size: int


@dataclass(frozen=True)
class Manifest:
    revision: str
    artifacts: tuple[ArtifactRecord, ...]

    def to_dict(self) -> dict:
        return {
            "revision": self.revision,
            "artifacts": [asdict(a) for a in self.artifacts],
        }


def sha256_bytes(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def sha256_file(path: Path) -> ArtifactRecord:
    data = path.read_bytes()
    return ArtifactRecord(path=str(path), sha256=sha256_bytes(data), size=len(data))


def build_manifest(paths: Iterable[Path], *, revision: str) -> Manifest:
    records = tuple(sha256_file(Path(p)) for p in paths)
    return Manifest(revision=revision, artifacts=records)


def dumps_manifest(manifest: Manifest) -> str:
    return json.dumps(manifest.to_dict(), indent=2, sort_keys=True) + "\n"


def license_inventory(entries: Iterable[tuple[str, str]]) -> str:
    """Build a deterministic dependency license inventory from (name, license) pairs."""
    rows = sorted((name, lic) for name, lic in entries)
    lines = ["# Dependency license inventory", ""]
    for name, lic in rows:
        lines.append(f"- `{name}`: {lic}")
    lines.append("")
    return "\n".join(lines)


def validate_evidence(
    *,
    manifest_sha_by_path: dict[str, str],
    observed_sha_by_path: dict[str, str],
) -> tuple[bool, tuple[str, ...]]:
    problems: list[str] = []
    for path, expected in sorted(manifest_sha_by_path.items()):
        got = observed_sha_by_path.get(path)
        if got is None:
            problems.append(f"missing artifact: {path}")
        elif got != expected:
            problems.append(f"checksum mismatch: {path}")
    for path in sorted(observed_sha_by_path):
        if path not in manifest_sha_by_path:
            problems.append(f"unexpected artifact: {path}")
    return not problems, tuple(problems)
