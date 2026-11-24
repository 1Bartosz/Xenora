# SPDX-License-Identifier: GPL-3.0-or-later
"""Claim concurrency: one group, pending runs cancelled by newer ones, replay of unanswered commands."""
import copy
import unittest

from helpers import CFG, NOW, issue, state
from agent_queue import replay as R
from agent_queue.decide import decide
from agent_queue.lease import GRANT

BOT = "github-actions[bot]"


def cmd(cid, user, text, n, at="2026-01-10T11:00:00Z", pr=False):
    return {"id": cid, "user": user, "body": text, "created_at": at, "issue": n, "pull_request": pr}


def queue_state(comments):
    # #1 and #2 share src/gpu/a.rs; #3 is independent.
    return state(issue(1), issue(2), issue(3, files="src/gpu/b.rs"), recent_comments=comments)


def granted(actions):
    return sorted((int(m.group(1)), m.group(2)) for a in actions for m in [GRANT.search(a["comment"])] if m)


def simulate(posted, run_once):
    """GitHub concurrency group with cancel-in-progress: false. Each comment queues a run; while a
    run is in progress one run may wait, and a newly queued run cancels the waiting one. Here every
    comment arrives while the first run is still in progress. Returns the posted bot replies."""
    st = queue_state([])
    issues = st["issues"]
    timeline, replies = [], []
    in_progress, pending = posted[0], None
    for c in posted[1:]:
        pending = c                      # an older pending run, if any, is cancelled here
    for trigger, visible in ((in_progress, posted[:1]), (pending, posted)):
        if trigger is None:
            continue
        view = {"issues": issues, "prs": [], "task_comments": {},
                "recent_comments": visible + timeline}
        actions = run_once(trigger, view)
        for a in actions:
            R._apply(st, a)
            timeline.append({"id": 10_000 + len(timeline), "user": BOT, "body": a["comment"],
                             "created_at": "2026-01-10T11:30:00Z", "issue": a["issue"], "pull_request": False})
        replies.extend(actions)
    return replies


def event_only(trigger, view):
    """The previous design: a run decides only the comment that started it."""
    return decide({"kind": "issue", "number": trigger["issue"], "actor": trigger["user"], "body": trigger["body"]},
                  view, CFG)


def with_replay(trigger, view):
    return R.replay(view, CFG, NOW)


POSTED = [
    cmd(1, "maint", "/claim", 3, "2026-01-10T11:00:00Z"),
    cmd(2, "gpuer", "/claim", 1, "2026-01-10T11:00:01Z"),
    cmd(3, "maint", "/claim", 2, "2026-01-10T11:00:02Z"),   # overlaps #1 on src/gpu/a.rs
    cmd(4, "gpuer", "/progress parser done", 1, "2026-01-10T11:00:03Z"),
]


class PendingCancellation(unittest.TestCase):
    def test_deciding_only_the_trigger_loses_cancelled_commands(self):
        # Runs 2 and 3 are cancelled while pending: gpuer's claim on #1 is never answered.
        self.assertEqual(granted(simulate(POSTED, event_only)), [(3, "maint")])

    def test_replay_answers_every_command_of_cancelled_runs(self):
        replies = simulate(POSTED, with_replay)
        self.assertEqual(granted(replies), [(1, "gpuer"), (3, "maint")])
        refused = [a for a in replies if a["issue"] == 2]
        self.assertEqual(len(refused), 1)
        self.assertIn("file overlap with claimed task #1", refused[0]["comment"])
        answered = R.answered([{"user": BOT, "body": a["comment"]} for a in replies])
        self.assertEqual(answered, {1, 2, 3, 4})


class Replay(unittest.TestCase):
    def run_replay(self, comments, st=None):
        return R.replay(st or queue_state(comments), CFG, NOW)

    def test_overlapping_claims_in_one_run_are_decided_in_order(self):
        actions = self.run_replay([cmd(8, "maint", "/claim", 2, "2026-01-10T11:00:05Z"),
                                   cmd(7, "gpuer", "/claim", 1, "2026-01-10T11:00:05Z")])
        # Same second: the lower comment id is first.
        self.assertEqual(granted(actions), [(1, "gpuer")])
        self.assertIn("file overlap with claimed task #1", actions[1]["comment"])
        self.assertIn(R.done_marker(8), actions[1]["comment"])

    def test_out_of_order_listing_is_decided_in_comment_order(self):
        # Listed newest first: the earlier claim on the shared file still wins.
        actions = self.run_replay([cmd(9, "maint", "/claim", 2, "2026-01-10T11:00:09Z"),
                                   cmd(5, "gpuer", "/claim", 1, "2026-01-10T11:00:01Z")])
        self.assertEqual(granted(actions), [(1, "gpuer")])
        self.assertEqual([a["issue"] for a in actions], [1, 2])

    def test_duplicate_events_are_decided_once(self):
        c = cmd(5, "gpuer", "/claim", 1)
        actions = self.run_replay([c, dict(c), cmd(6, "maint", "/claim", 2, "2026-01-10T11:00:02Z")])
        self.assertEqual(granted(actions), [(1, "gpuer")])
        self.assertEqual(sum(R.done_marker(5) in a["comment"] for a in actions), 1)

    def test_no_burst_order_grants_overlapping_claims(self):
        # Every arrival order of a burst of claims on #1 and #2 (shared file) grants exactly one of them.
        import itertools
        burst = [cmd(1, "gpuer", "/claim", 1), cmd(2, "maint", "/claim", 2), cmd(3, "maint", "/claim", 1),
                 cmd(4, "gpuer", "/claim", 2), cmd(5, "maint", "/claim", 3)]
        for order in itertools.permutations(range(5)):
            comments = [dict(burst[i], created_at="2026-01-10T11:00:%02dZ" % k) for k, i in enumerate(order)]
            g = granted(self.run_replay(comments))
            self.assertEqual(len([x for x in g if x[0] in (1, 2)]), 1, (order, g))
            self.assertIn((3, "maint"), g)

    def test_answered_commands_are_not_decided_again(self):
        first = self.run_replay([cmd(1, "gpuer", "/claim", 1)])
        st = queue_state([cmd(1, "gpuer", "/claim", 1),
                          {"id": 2, "user": BOT, "body": first[0]["comment"], "created_at": "2026-01-10T11:01:00Z",
                           "issue": 1, "pull_request": False}])
        for a in first:
            R._apply(st, a)
        self.assertEqual(R.replay(st, CFG, NOW), [])

    def test_done_markers_count_only_from_the_bot(self):
        forged = {"id": 2, "user": "rt", "body": R.done_marker(1), "created_at": "2026-01-10T11:01:00Z",
                  "issue": 1, "pull_request": False}
        self.assertEqual(granted(self.run_replay([cmd(1, "gpuer", "/claim", 1), forged])), [(1, "gpuer")])

    def test_ordinary_pr_and_old_comments_start_nothing(self):
        self.assertEqual(self.run_replay([
            cmd(1, "gpuer", "looks good to me", 1),
            cmd(2, "gpuer", "/claim", 1, pr=True),
            cmd(3, "maint", "/verified abcdef1", 1),
            cmd(4, "gpuer", "/claim", 1, "2026-01-09T11:00:00Z"),  # older than lease_hours
        ]), [])

    def test_state_is_not_modified(self):
        st = queue_state([cmd(1, "gpuer", "/claim", 1)])
        before = copy.deepcopy(st)
        self.assertEqual(len(R.replay(st, CFG, NOW)), 1)
        self.assertEqual(st, before)


if __name__ == "__main__":
    unittest.main()
