# SPDX-License-Identifier: GPL-3.0-or-later
import json
import unittest

from qa.handoff import HandoffReport, WorkItem, conflict_paths, handoff_markdown, progress_summary
from qa.regression import compare_reports, has_new_regressions
from qa.repo_health import check_ignored_artifacts_tracked, check_large_generated_paths, format_health


class RegressionHandoffHealth(unittest.TestCase):
    def test_compare_reports(self):
        prev = {"suites": [{"name": "a", "status": "passed", "kind": "cpu"},
                           {"name": "b", "status": "failed", "kind": "cpu"}]}
        cur = {"suites": [{"name": "a", "status": "failed", "kind": "cpu"},
                          {"name": "b", "status": "passed", "kind": "cpu"}]}
        diff = compare_reports(prev, cur)
        self.assertEqual(diff.newly_failed[0].name, "a")
        self.assertEqual(diff.newly_passed[0].name, "b")
        self.assertTrue(has_new_regressions(diff))
        self.assertIn("a", json.dumps(prev))

    def test_handoff_and_conflicts(self):
        items = [
            WorkItem(1, "one", "claimed", ("scripts/qa/",)),
            WorkItem(2, "two", "claimed", ("scripts/qa/x.py",)),
            WorkItem(3, "three", "done", ("scripts/release/",)),
        ]
        self.assertTrue(conflict_paths(items))
        text = progress_summary(items)
        self.assertIn("#1", text)
        report = HandoffReport("alice", "deadbeef", tuple(items), ("gpu not measured",))
        md = handoff_markdown(report)
        self.assertIn("Limitations", md)
        self.assertIn("gpu not measured", md)

    def test_repo_health(self):
        bad = check_ignored_artifacts_tracked(["scripts/qa/a.py", "x/__pycache__/y.pyc"])
        self.assertFalse(bad.ok)
        gen = check_large_generated_paths(["artifacts/foo.json", "scripts/qa/a.py"])
        self.assertFalse(gen.ok)
        self.assertIn("Repository health", format_health([bad, gen]))


if __name__ == "__main__":
    unittest.main()
