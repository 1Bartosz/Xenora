# SPDX-License-Identifier: GPL-3.0-or-later
import json
import os
import subprocess
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
ENV = {**os.environ, "PYTHONPATH": str(ROOT / "scripts")}


def run(*args: str) -> subprocess.CompletedProcess:
    return subprocess.run(
        ["python3", "-m", *args],
        cwd=ROOT,
        capture_output=True,
        text=True,
        env=ENV,
    )


class CliSmoke(unittest.TestCase):
    def test_orchestrate_shlex_and_rdna2_inventory(self):
        r = run("qa", "orchestrate", "--name", "ok", "--run", "python3 -c 'print(1)'")
        self.assertEqual(r.returncode, 0, r.stderr)
        payload = json.loads(r.stdout)
        self.assertEqual(payload["counts"]["passed"], 1)

        inv = run("qa", "rdna2", "inventory")
        self.assertEqual(inv.returncode, 0, inv.stderr)
        data = json.loads(inv.stdout)
        self.assertGreater(data["identified"], data["declared"])

        compact = run("qa", "rdna2", "candidates", "--status", "reuse-primitives", "--limit", "2", "--compact")
        self.assertEqual(compact.returncode, 0, compact.stderr)
        self.assertIn("candidates=2", compact.stdout)

        recon = run("qa", "rdna2", "reconcile", "--compact")
        self.assertEqual(recon.returncode, 0, recon.stderr)
        self.assertIn("reconcile_ok=", recon.stdout)

        with tempfile.TemporaryDirectory() as tmp:
            out = Path(tmp) / "manifests.json"
            man = run(
                "qa", "rdna2", "manifests",
                "--status", "reuse-primitives",
                "--limit", "2",
                "--out", str(out),
            )
            self.assertEqual(man.returncode, 0, man.stderr)
            payload = json.loads(out.read_text(encoding="utf-8"))
            self.assertEqual(payload["count"], 2)
            self.assertIn("not implemented", payload["note"].lower())

    def test_release_readiness_cli_negative(self):
        r = run(
            "release", "readiness",
            "--revision", "deadbeef",
            "--lipo", "x86_64",
            "--binary", "/tmp/x",
            "--otool-l", "/dev/null",
        )
        # empty otool file + x86_64 should fail readiness
        self.assertNotEqual(r.returncode, 0)


if __name__ == "__main__":
    unittest.main()
