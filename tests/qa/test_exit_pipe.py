# SPDX-License-Identifier: GPL-3.0-or-later
import unittest

from qa.exit_pipe import classify_status, run_preserving_status


class ExitPipe(unittest.TestCase):
    def test_preserves_nonzero_status(self):
        result = run_preserving_status(["python3", "-c", "import sys; sys.exit(7)"])
        self.assertEqual(result.returncode, 7)
        self.assertEqual(classify_status(result.returncode), "failed")

    def test_preserves_zero_status(self):
        result = run_preserving_status(["python3", "-c", "print('ok')"])
        self.assertEqual(result.returncode, 0)
        self.assertEqual(classify_status(result.returncode), "passed")
        self.assertIn("ok", result.stdout)

    def test_failure_not_masked_by_later_success_semantics(self):
        # The helper returns the command status directly; callers must not pipe
        # through shell grep. This asserts the contract used by orchestrate.
        failed = run_preserving_status(["python3", "-c", "import sys; print('needle'); sys.exit(3)"])
        self.assertEqual(failed.returncode, 3)
        self.assertIn("needle", failed.stdout)


if __name__ == "__main__":
    unittest.main()
