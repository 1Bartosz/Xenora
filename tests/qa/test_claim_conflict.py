# SPDX-License-Identifier: GPL-3.0-or-later
import unittest

from qa.claim_conflict import ClaimSnapshot, format_conflicts, predict_conflicts


class ClaimConflict(unittest.TestCase):
    def test_predicts_directory_overlap(self):
        held = [
            ClaimSnapshot(10, "alice", ("scripts/qa/",), True),
            ClaimSnapshot(11, "bob", ("scripts/release/",), True),
        ]
        hits = predict_conflicts(["scripts/qa/orchestrate.py"], held)
        self.assertEqual(len(hits), 1)
        self.assertEqual(hits[0].held_by, 10)
        self.assertIn("overlaps claimed #10", format_conflicts(hits))

    def test_ignores_unclaimed_and_disjoint(self):
        held = [
            ClaimSnapshot(12, "", ("scripts/qa/",), False),
            ClaimSnapshot(13, "carol", ("docs/testing/infrastructure/",), True),
        ]
        hits = predict_conflicts(["scripts/release/macho.py"], held)
        self.assertEqual(hits, ())
        self.assertEqual(format_conflicts(hits), "no file conflicts predicted\n")


if __name__ == "__main__":
    unittest.main()
