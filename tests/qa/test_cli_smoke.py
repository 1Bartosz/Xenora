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
