# SPDX-License-Identifier: GPL-3.0-or-later
"""Exercise the native audit's Cargo JSON reader under an ASCII locale."""

import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[2]
SCRIPT = (ROOT / "scripts/audit/native.sh").read_text(encoding="utf-8")
# Run the actual reader, stopping before the native executable inspection.
READER = SCRIPT.split("python3 - <<'PY'\n", 1)[1].split(
    "\nif not executables:", 1
)[0]


class NativeJsonTests(unittest.TestCase):
    def run_reader(self, streams):
        with tempfile.TemporaryDirectory() as directory:
            artifacts = Path(directory) / "artifacts"
            artifacts.mkdir()
            for name, payload in zip(
                ("native-build.jsonl", "native-tests.jsonl"), streams, strict=True
            ):
                (artifacts / name).write_bytes(payload)
            return subprocess.run(
                [
                    sys.executable,
                    "-c",
                    "import locale\n"
                    "assert locale.getencoding().lower() in ('ascii', 'us-ascii')\n"
                    + READER
                    + "\nprint(json.dumps(sorted(executables)))\n",
                ],
                cwd=directory,
                env={
                    **os.environ,
                    "LC_ALL": "C",
                    "PYTHONCOERCECLOCALE": "0",
                    "PYTHONUTF8": "0",
                },
                capture_output=True,
                encoding="utf-8",
                check=False,
            )

    def test_utf8_in_each_cargo_stream(self):
        for stream in (0, 1):
            with self.subTest(stream=stream):
                entry = {
                    "executable": "/synthetic/caf\u00e9/bin",
                    "filenames": ["/synthetic/caf\u00e9/lib.dylib"],
                    "message": {"rendered": "synthetic diagnostic: \u03bb"},
                }
                streams = [b'{}\n', b'{}\n']
                streams[stream] = (json.dumps(entry, ensure_ascii=False) + "\n").encode(
                    "utf-8"
                )
                result = self.run_reader(streams)
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(
                    json.loads(result.stdout),
                    ["/synthetic/caf\u00e9/bin", "/synthetic/caf\u00e9/lib.dylib"],
                )

    def test_ascii_streams_keep_deduplicated_artifacts(self):
        entry = b'{"executable":"/synthetic/bin","filenames":["/synthetic/lib.so","/synthetic/lib.rlib"]}\n'
        result = self.run_reader([entry, entry])
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(
            json.loads(result.stdout), ["/synthetic/bin", "/synthetic/lib.so"]
        )

    def test_invalid_utf8_is_rejected(self):
        result = self.run_reader([b'{"executable":"\xff"}\n', b'{}\n'])
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("UnicodeDecodeError", result.stderr)


if __name__ == "__main__":
    unittest.main()
