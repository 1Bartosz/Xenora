# SPDX-License-Identifier: GPL-3.0-or-later
import json
import unittest

from qa.orchestrate import (
    SuiteSpec,
    dumps_report,
    failure_log_excerpt,
    run_suites,
    separate_cpu_gpu,
)


class Orchestrate(unittest.TestCase):
    def test_records_pass_and_fail_with_durations(self):
        report = run_suites([
            SuiteSpec("ok", ("python3", "-c", "print(1)"), "cpu"),
            SuiteSpec("bad", ("python3", "-c", "import sys; sys.exit(2)"), "cpu"),
        ])
        self.assertEqual(report.passed, 1)
        self.assertEqual(report.failed, 1)
        self.assertGreaterEqual(report.suites[0].duration_seconds, 0.0)
        self.assertEqual(report.overall_returncode(), 2)
        payload = json.loads(dumps_report(report))
        self.assertEqual(payload["counts"]["failed"], 1)

    def test_stop_on_fail_marks_unexecuted(self):
        report = run_suites(
            [
                SuiteSpec("bad", ("python3", "-c", "import sys; sys.exit(5)"), "cpu"),
                SuiteSpec("later", ("python3", "-c", "print('nope')"), "gpu"),
            ],
            stop_on_fail=True,
        )
        self.assertEqual(report.suites[1].status, "unexecuted")
        self.assertEqual(report.unexecuted, 1)

    def test_ignored_and_cpu_gpu_split(self):
        report = run_suites([
            SuiteSpec("skip", ("true",), "ignored"),
            SuiteSpec("cpu", ("python3", "-c", "print('c')"), "cpu"),
            SuiteSpec("gpu", ("python3", "-c", "print('g')"), "gpu"),
        ])
        self.assertEqual(report.ignored, 1)
        buckets = separate_cpu_gpu(report)
        self.assertEqual(len(buckets["cpu"]), 1)
        self.assertEqual(len(buckets["gpu"]), 1)

    def test_failure_excerpt_includes_reproduction(self):
        report = run_suites([
            SuiteSpec("bad", ("python3", "-c", "import sys; sys.stderr.write('boom\\n'); sys.exit(1)"), "cpu"),
        ])
        text = failure_log_excerpt(report)
        self.assertIn("reproduce:", text)
        self.assertIn("boom", text)


if __name__ == "__main__":
    unittest.main()
