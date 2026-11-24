# SPDX-License-Identifier: GPL-3.0-or-later
import json
import tempfile
import unittest
from pathlib import Path

from release.manifest import (
    build_manifest,
    dumps_manifest,
    license_inventory,
    sha256_bytes,
    validate_evidence,
)


class Manifest(unittest.TestCase):
    def test_checksums_and_validation(self):
        self.assertEqual(sha256_bytes(b"abc"),
                         "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad")
        with tempfile.TemporaryDirectory() as tmp:
            p = Path(tmp) / "a.bin"
            p.write_bytes(b"hello")
            manifest = build_manifest([p], revision="rev1")
            payload = json.loads(dumps_manifest(manifest))
            self.assertEqual(payload["revision"], "rev1")
            expected = {payload["artifacts"][0]["path"]: payload["artifacts"][0]["sha256"]}
            ok, problems = validate_evidence(
                manifest_sha_by_path=expected,
                observed_sha_by_path=expected,
            )
            self.assertTrue(ok)
            self.assertEqual(problems, ())
            bad, problems = validate_evidence(
                manifest_sha_by_path=expected,
                observed_sha_by_path={payload["artifacts"][0]["path"]: "0" * 64},
            )
            self.assertFalse(bad)
            self.assertTrue(problems)

    def test_license_inventory_sorted(self):
        text = license_inventory([("zeta", "MIT"), ("alpha", "Apache-2.0")])
        self.assertLess(text.index("alpha"), text.index("zeta"))
        self.assertIn("Apache-2.0", text)


if __name__ == "__main__":
    unittest.main()
