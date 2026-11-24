# SPDX-License-Identifier: GPL-3.0-or-later
"""Read-only conflict prediction before a claim is posted.

Uses the same pure `claim_problem` rules as the bot, without applying labels or
posting comments. Contributors can evaluate overlap, area, dependency and
readiness failures locally before they edit files.
"""
import unittest

from agent_queue.decide import claim_problem, decide, task_index

from helpers import CFG, event, issue, state


class ConflictPredict(unittest.TestCase):
    def test_predicts_overlap_without_mutating_state(self):
        held = issue(1, labels=("agent:claimed", "claimed-by:gpuer", "area:gpu"),
                     area="gpu", files="src/gpu/**")
        candidate = issue(2, labels=("agent:ready", "area:gpu"),
                          area="gpu", files="src/gpu/a.rs")
        snap = state(held, candidate, task_comments={
            "1": [{"user": "github-actions[bot]", "body": "Granted to gpuer.\n<!-- agent-queue:grant -->",
                   "created_at": "2026-01-10T09:00:00Z"}],
        })
        before = [dict(i) for i in snap["issues"]]
        tasks = task_index(snap)
        problem = claim_problem(tasks[2], tasks, CFG, "gpuer")
        self.assertIn("file overlap with claimed task #1", problem)
        self.assertEqual(snap["issues"], before)

    def test_predicts_ready_when_no_conflict(self):
        ready = issue(3, labels=("agent:ready", "area:qa"), area="qa", files="tests/qa/**")
        tasks = task_index(state(ready))
        self.assertEqual(claim_problem(tasks[3], tasks, CFG, "qaer"), "")

    def test_predicts_wrong_area_and_open_dependency(self):
        dep = issue(4, labels=("agent:ready", "area:cpu"), area="cpu", files="crates/loader/**")
        blocked = issue(5, labels=("agent:ready", "area:qa"), area="qa",
                        files="docs/testing/**", deps="#4")
        tasks = task_index(state(dep, blocked))
        self.assertIn("not allowed to claim area:cpu", claim_problem(tasks[4], tasks, CFG, "qaer"))
        self.assertIn("open dependencies: #4", claim_problem(tasks[5], tasks, CFG, "qaer"))

    def test_predict_agrees_with_live_claim_decision(self):
        held = issue(6, labels=("agent:claimed", "claimed-by:maint", "area:shared"),
                     area="shared", files="docs/interfaces.md")
        ready = issue(7, labels=("agent:ready", "area:shared"),
                      area="shared", files="docs/interfaces.md")
        snap = state(held, ready, task_comments={
            "6": [{"user": "github-actions[bot]", "body": "Granted.\n<!-- agent-queue:grant -->",
                   "created_at": "2026-01-10T09:00:00Z"}],
        })
        tasks = task_index(snap)
        predicted = claim_problem(tasks[7], tasks, CFG, "maint")
        actions = decide(event("maint", "/claim", n=7), snap, CFG)
        self.assertTrue(predicted)
        self.assertEqual(len(actions), 1)
        self.assertIn(predicted, actions[0]["comment"])
        self.assertEqual(actions[0].get("add_labels"), [])


if __name__ == "__main__":
    unittest.main()
