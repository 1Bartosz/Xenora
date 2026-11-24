# SPDX-License-Identifier: GPL-3.0-or-later
import json
import tempfile
import unittest
from pathlib import Path

from qa.orchestrate import SuiteSpec
from qa.verify import dumps_evidence, evidence_is_stale, fingerprint_sources, load_evidence, run_profile

ROOT = Path(__file__).resolve().parents[2]


class VerifyPipeline(unittest.TestCase):
    def test_real_unittest_profile_slice(self):
        # Real Xenora command, not a mock: run a tiny existing unittest module.
        evidence = run_profile(
            "custom",
            root=ROOT,
            revision="testrev",
            override_specs=[
                SuiteSpec(
                    "env-check-tests",
                    ("python3", "-m", "unittest", "tests.qa.test_env_check"),
                    "cpu",
                ),
                SuiteSpec(
                    "deliberate-fail",
                    ("python3", "-c", "import sys; sys.exit(9)"),
                    "cpu",
                ),
                SuiteSpec(
                    "should-skip",
                    ("python3", "-c", "print('nope')"),
                    "cpu",
                ),
            ],
            stop_on_fail=True,
        )
        self.assertEqual(evidence.suites[0]["status"], "passed")
        self.assertEqual(evidence.suites[1]["status"], "failed")
        self.assertEqual(evidence.suites[1]["returncode"], 9)
        self.assertEqual(evidence.suites[2]["status"], "unexecuted")
        self.assertEqual(evidence.overall_returncode, 9)
        self.assertEqual(evidence.skipped["unexecuted"], 1)
        text = dumps_evidence(evidence)
        loaded = load_evidence(text)
        self.assertEqual(loaded.revision, "testrev")

    def test_stale_evidence_when_sources_change(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            f = root / "scripts" / "a.py"
            f.parent.mkdir(parents=True)
            f.write_text("print(1)\n", encoding="utf-8")
            evidence = run_profile(
                "custom",
                root=root,
                revision="r1",
                override_specs=[SuiteSpec("ok", ("python3", "-c", "print(1)"), "cpu")],
            )
            self.assertFalse(evidence_is_stale(evidence, root))
            f.write_text("print(2)\n", encoding="utf-8")
            self.assertTrue(evidence_is_stale(evidence, root))
            self.assertNotEqual(evidence.source_fingerprint, fingerprint_sources(root))


if __name__ == "__main__":
    unittest.main()
