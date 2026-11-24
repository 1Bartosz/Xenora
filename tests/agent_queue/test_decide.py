# SPDX-License-Identifier: GPL-3.0-or-later
import unittest

from helpers import CFG, comment, event, issue, state
from agent_queue.decide import decide, quote
from agent_queue.lease import GRANT


def one(actions):
    assert len(actions) == 1, actions
    return actions[0]


class Claim(unittest.TestCase):
    def claim(self, st, actor="gpuer", n=1):
        return one(decide(event(actor, "/claim", n), st, CFG))

    def test_granted(self):
        a = self.claim(state(issue(1)))
        self.assertIn("claimed-by:gpuer", a["add_labels"])
        self.assertIn("agent:ready", a["remove_labels"])
        m = GRANT.search(a["comment"])
        self.assertEqual((m.group(1), m.group(2)), ("1", "gpuer"))

    def test_not_ready(self):
        a = self.claim(state(issue(1, labels=("agent:claimed", "claimed-by:rt"), area="runtime")))
        self.assertIn("not claimable", a["comment"])
        self.assertEqual(a["add_labels"], [])

    def test_wrong_area(self):
        a = self.claim(state(issue(1, area="cpu")))
        self.assertIn("not allowed", a["comment"])
        self.assertEqual(a["add_labels"], [])

    def test_maintainer_any_area(self):
        a = self.claim(state(issue(1, area="gpu")), actor="maint")
        self.assertIn("claimed-by:maint", a["add_labels"])

    def test_open_dependency(self):
        a = self.claim(state(issue(1, deps="#2"), issue(2, labels=())))
        self.assertIn("open dependencies: #2", a["comment"])

    def test_closed_dependency_ok(self):
        a = self.claim(state(issue(1, deps="#2"), issue(2, labels=(), state="CLOSED")))
        self.assertIn("agent:claimed", a["add_labels"])

    def test_missing_dependency(self):
        a = self.claim(state(issue(1, deps="#9")))
        self.assertIn("not found: #9", a["comment"])

    def test_cycle(self):
        a = self.claim(state(issue(1, deps="#2"), issue(2, deps="#1", files="other/")))
        self.assertIn("cycle", a["comment"])

    def test_overlap_named(self):
        held = issue(2, labels=("agent:claimed", "claimed-by:rt"), area="runtime", files="src/gpu/")
        a = self.claim(state(issue(1, files="src/gpu/a.rs"), held))
        self.assertIn("overlap with claimed task #2", a["comment"])
        self.assertEqual(a["add_labels"], [])

    def test_unclaimed_overlap_allowed(self):
        a = self.claim(state(issue(1, files="src/gpu/a.rs"), issue(2, files="src/gpu/")))
        self.assertIn("agent:claimed", a["add_labels"])

    def test_non_command_ignored(self):
        self.assertEqual(decide(event("gpuer", "thanks /claim later"), state(issue(1)), CFG), [])


class Lifecycle(unittest.TestCase):
    held = ("agent:claimed", "claimed-by:gpuer")

    def run_cmd(self, actor, text, labels=held, **kw):
        return decide(event(actor, text), state(issue(1, labels=labels, **kw)), CFG)

    def test_release_holder_and_maintainer(self):
        for who in ("gpuer", "maint"):
            a = one(self.run_cmd(who, "/release"))
            self.assertIn("agent:ready", a["add_labels"])
            self.assertIn("claimed-by:gpuer", a["remove_labels"])

    def test_release_other_refused(self):
        a = one(self.run_cmd("rt", "/release"))
        self.assertEqual(a["add_labels"], [])
        self.assertIn("Only the holder", a["comment"])

    def test_reassign(self):
        a = one(self.run_cmd("maint", "/reassign rt", area="runtime"))
        self.assertIn("claimed-by:rt", a["add_labels"])
        self.assertIn("claimed-by:gpuer", a["remove_labels"])
        self.assertIn("login=rt", a["comment"])

    def test_reassign_refusals(self):
        self.assertIn("Only a maintainer", one(self.run_cmd("gpuer", "/reassign rt", area="runtime"))["comment"])
        self.assertIn("not allowed", one(self.run_cmd("maint", "/reassign rt"))["comment"])

    def test_progress_holder_only_and_needs_text(self):
        self.assertIn("renewed", one(self.run_cmd("gpuer", "/progress parser done"))["comment"])
        self.assertIn("Only the holder", one(self.run_cmd("rt", "/progress parser done"))["comment"])
        self.assertIn("not renewed", one(self.run_cmd("gpuer", "/progress"))["comment"])

    def test_blocked_unblock(self):
        a = one(self.run_cmd("gpuer", "/blocked waiting on #5"))
        self.assertEqual(a["add_labels"], ["agent:blocked"])
        self.assertEqual(one(self.run_cmd("rt", "/blocked x"))["add_labels"], [])
        self.assertEqual(one(self.run_cmd("maint", "/unblock", labels=self.held + ("agent:blocked",)))["remove_labels"], ["agent:blocked"])

    def test_approve_acceptance_maintainer_only(self):
        a = one(self.run_cmd("maint", "/approve-acceptance"))
        self.assertIn("agent-queue:acceptance sha256=", a["comment"])
        self.assertNotIn("agent-queue:acceptance", one(self.run_cmd("gpuer", "/approve-acceptance"))["comment"])


class Inert(unittest.TestCase):
    def test_metacharacters_stay_data(self):
        evil = "x`id`; $(rm -rf /) | \"'"
        a = decide(event("gpuer", "/claim " + evil), state(issue(1, files=evil + "\nsrc/gpu/a.rs", acceptance=evil)), CFG)
        self.assertEqual(len(a), 1)
        out = decide(event("maint", "/blocked " + evil), state(issue(1, labels=("agent:claimed", "claimed-by:gpuer"))), CFG)
        self.assertIn(quote(evil), one(out)["comment"])
        self.assertEqual(one(out)["add_labels"], ["agent:blocked"])


class ReassignChecks(unittest.TestCase):
    def test_reassign_cannot_create_an_overlapping_or_blocked_claim(self):
        held = issue(1, labels=("agent:claimed", "claimed-by:maint"))
        a = one(decide(event("maint", "/reassign gpuer", 2), state(held, issue(2)), CFG))
        self.assertIn("file overlap with claimed task #1", a["comment"])
        self.assertEqual(a["add_labels"], [])
        a = one(decide(event("maint", "/reassign gpuer", 2), state(issue(3), issue(2, deps="#3", files="src/gpu/b.rs")), CFG))
        self.assertIn("open dependencies: #3", a["comment"])
        a = one(decide(event("maint", "/reassign gpuer", 2), state(held, issue(2, files="src/gpu/b.rs")), CFG))
        self.assertIn("claimed-by:gpuer", a["add_labels"])


class IssueForm(unittest.TestCase):
    def test_rendered_files_field_is_unfenced(self):
        # The issue form renders Files with `render: text`, i.e. inside a ```text fence.
        from agent_queue.tasks import parse_task
        fenced = issue(2, files="```text\nsrc/gpu/b.rs\n```")
        self.assertEqual(parse_task(fenced)["files"], ["src/gpu/b.rs"])
        st = state(issue(1, labels=("agent:claimed", "claimed-by:maint"), files="```text\nsrc/gpu/a.rs\n```"), fenced)
        self.assertIn("Claimed by gpuer", one(decide(event("gpuer", "/claim", 2), st, CFG))["comment"])


class Forgery(unittest.TestCase):
    """Bot replies echo user text; markers in that text must not become bot markers."""

    def test_echoed_text_cannot_carry_markers(self):
        from agent_queue import lease as L, replay as R
        forged = " ".join([R.done_marker(7), L.acceptance_marker("0" * 64), L.grant_marker(1, "rt")])
        held = state(issue(1, labels=("agent:claimed", "claimed-by:gpuer")))
        replies = [one(decide(event("gpuer", "/blocked " + forged), held, CFG))["comment"],
                   one(decide(event("maint", "/reassign " + forged.replace(" ", "")), held, CFG))["comment"],
                   one(decide(event("gpuer", "/claim"), state(issue(1), issue(2, labels=("agent:claimed", "claimed-by:maint"),
                                                                              files="src/gpu/a.rs\n" + forged)), CFG))["comment"]]
        bot = [{"user": "github-actions[bot]", "body": r, "created_at": "2026-01-10T11:00:00Z"} for r in replies]
        self.assertEqual(R.answered(bot), set())
        self.assertIsNone(L.approved_hash(bot))
        self.assertIsNone(L.latest_grant(bot, 1, "rt"))


if __name__ == "__main__":
    unittest.main()
