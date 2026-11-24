# SPDX-License-Identifier: GPL-3.0-or-later
import json
import tempfile
import unittest
from pathlib import Path

from release.readiness import (
    build_report,
    build_sbom,
    collect_artifacts,
    dumps_report,
    inspect_app,
    inspect_binary,
    require_evidence_files,
)


class Readiness(unittest.TestCase):
    def test_binary_and_app_negative_paths(self):
        bad = inspect_binary(
            path="/tmp/x",
            lipo_archs="x86_64",
            otool_l="/tmp/x:\n\t@rpath/libvulkan.dylib\n",
        )
        self.assertTrue(any(not f.ok for f in bad))
        with tempfile.TemporaryDirectory() as tmp:
            app = Path(tmp) / "Xenora.app"
            findings = inspect_app(app)
            self.assertFalse(findings[0].ok)

    def test_sbom_artifacts_and_report(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            blob = root / "a.bin"
            blob.write_bytes(b"hello")
            evidence = root / "verify.json"
            evidence.write_text("{}", encoding="utf-8")
            artifacts, art_findings = collect_artifacts([blob], revision="abc1234")
            sbom = build_sbom([("serde", "1.0", "MIT"), ("libc", "0.2", "MIT OR Apache-2.0")])
            report = build_report(
                revision="abc1234",
                binary_findings=inspect_binary(
                    path=str(blob), lipo_archs="arm64", otool_l=f"{blob}:\n\t/usr/lib/libSystem.B.dylib\n"
                ),
                app_findings=[],
                artifacts=artifacts,
                artifact_findings=art_findings,
                evidence_findings=require_evidence_files([evidence]),
                sbom=sbom,
            )
            self.assertTrue(report.ok)
            payload = json.loads(dumps_report(report))
            self.assertEqual(len(payload["sbom"]), 2)
            empty = build_report(revision="abc1234", sbom=[])
            self.assertFalse(empty.ok)


if __name__ == "__main__":
    unittest.main()
