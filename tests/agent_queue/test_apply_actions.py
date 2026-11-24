# SPDX-License-Identifier: GPL-3.0-or-later
"""Regression for live smoke failure: comment-only actions must not call issue edit."""
import unittest
from unittest import mock

from agent_queue import ghio


class ApplyActions(unittest.TestCase):
    def test_comment_only_skips_issue_edit(self):
        calls = []

        def fake_gh(*args):
            calls.append(list(args))
            return ""

        with mock.patch.object(ghio, "gh", side_effect=fake_gh):
            ghio.apply_actions("o/r", [{
                "issue": 2,
                "add_labels": [],
                "remove_labels": [],
                "comment": "Claim refused for alice: file overlap with claimed task #1.",
            }])
        self.assertEqual(calls, [[
            "issue", "comment", "2", "-R", "o/r", "--body",
            "Claim refused for alice: file overlap with claimed task #1.",
        ]])

    def test_label_change_edits_then_comments(self):
        calls = []

        def fake_gh(*args):
            calls.append(list(args))
            return ""

        with mock.patch.object(ghio, "gh", side_effect=fake_gh):
            ghio.apply_actions("o/r", [{
                "issue": 1,
                "add_labels": ["agent:claimed", "claimed-by:alice"],
                "remove_labels": ["agent:ready"],
                "comment": "Claimed by alice.\n<!-- agent-queue:grant -->",
            }])
        self.assertEqual(calls[0][:2], ["label", "create"])
        self.assertEqual(calls[1][:2], ["label", "create"])
        self.assertEqual(calls[2], [
            "issue", "edit", "1", "-R", "o/r",
            "--add-label", "agent:claimed",
            "--add-label", "claimed-by:alice",
            "--remove-label", "agent:ready",
        ])
        self.assertEqual(calls[3][:3], ["issue", "comment", "1"])


if __name__ == "__main__":
    unittest.main()
