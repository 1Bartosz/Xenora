# SPDX-License-Identifier: GPL-3.0-or-later
import json
import unittest

from helpers import CFG, NOW, issue, state
from agent_queue import lease as L
from agent_queue.decide import decide, rerun_check
from agent_queue.prcheck import check_pr
from agent_queue.queue import summary

BOT = "github-actions[bot]"
SHA = "a" * 40
HELD = ("agent:claimed", "claimed-by:gpuer")


def pr(n=10, author="gpuer", files=("src/gpu/a.rs",), body="Closes #1", sha=SHA):
    return {"number": n, "author": author, "body": body, "head_sha": sha,
            "head_pushed_at": "2026-01-10T11:00:00Z", "files": list(files), "state": "OPEN"}


def run(p, comments=(), others=(), issues=None):
    st = state(*(issues or [issue(1, labels=HELD)]), prs=[p] + list(others),
               task_comments={"1": [{"user": BOT, "body": L.grant_marker(1, "gpuer"), "created_at": "2026-01-10T01:00:00Z"}]})
    st["pr"], st["pr_comments"] = p, list(comments)
    return check_pr(st, CFG, NOW)


def vc(user, sha=SHA[:12]):
    return {"user": user, "body": "/verified " + sha, "created_at": "2026-01-10T11:30:00Z"}


class PRChecks(unittest.TestCase):
    def test_clean_pr_passes(self):
        r = run(pr(), [vc("qaer")])
        self.assertTrue(r["ok"], r)
        self.assertEqual(r["verified_by"], ["qaer"])

    def test_verification_needs_no_bot_reply(self):
        # A /verified whose verify run was cancelled while pending has no bot reply; the check reads
        # the comments themselves, so the surviving run's re-run still counts it.
        r = run(pr(), [vc("qaer"), vc("maint"), {"user": BOT, "body": "Verification of %s recorded for maint." % SHA[:12],
                                                  "created_at": "2026-01-10T11:31:00Z"}])
        self.assertTrue(r["ok"], r)
        self.assertEqual(r["verified_by"], ["qaer", "maint"])

    def test_missing_link(self):
        r = run(pr(body="no link"), [vc("qaer")])
        self.assertFalse(r["ok"])
        self.assertIn("no linked task", r["problems"][0])

    def test_ownership_violation(self):
        r = run(pr(files=("src/gpu/a.rs", "src/cpu/x.rs")), [vc("qaer")])
        self.assertFalse(r["ok"])
        self.assertTrue(any("outside task #1" in p and "src/cpu/x.rs" in p for p in r["problems"]))

    def test_cross_pr_overlap(self):
        other = pr(n=11, author="rt", files=("src/gpu/a.rs",), body="Closes #2")
        r = run(pr(), [vc("qaer")], others=[other])
        self.assertTrue(any("overlaps open PR #11" in p for p in r["problems"]))

    def test_task_held_by_someone_else(self):
        r = run(pr(author="rt"), [vc("qaer")])
        self.assertTrue(any("not the pull request author" in p for p in r["problems"]))

    def test_unverified(self):
        r = run(pr())
        self.assertTrue(any("verification missing" in p for p in r["problems"]))

    def test_verification_by_author_or_holder_rejected(self):
        self.assertFalse(run(pr(), [vc("gpuer")])["ok"])
        # holder of the linked task who is not the author
        r = run(pr(author="maint"), [vc("gpuer")])
        self.assertEqual(r["verified_by"], [])

    def test_verification_by_ineligible_rejected(self):
        self.assertEqual(run(pr(), [vc("rt")])["verified_by"], [])

    def test_stale_sha_rejected(self):
        r = run(pr(), [vc("deadbeef00")])
        self.assertEqual(r["verified_by"], [])
        self.assertFalse(r["ok"])

    def test_lapsed_claim_and_stale_head(self):
        p = pr()
        p["head_pushed_at"] = "2026-01-08T00:00:00Z"
        st = state(issue(1, labels=HELD), prs=[p], task_comments={"1": [
            {"user": BOT, "body": L.grant_marker(1, "gpuer"), "created_at": "2026-01-08T00:00:00Z"}]})
        st["pr"], st["pr_comments"] = p, [vc("qaer")]
        self.assertTrue(any("lapsed" in x for x in check_pr(st, CFG, NOW)["problems"]))


class VerifiedCommand(unittest.TestCase):
    def go(self, actor, sha=SHA[:12], author="gpuer"):
        st = state(issue(1, labels=HELD))
        st["pr"] = pr(author=author)
        ev = {"kind": "pr", "number": 10, "actor": actor, "body": "/verified " + sha}
        return decide(ev, st, CFG)[0]["comment"]

    def test_outcomes(self):
        self.assertIn("recorded", self.go("qaer"))
        self.assertIn("author cannot", self.go("gpuer"))
        self.assertIn("holds the linked task", self.go("gpuer", author="maint"))
        self.assertIn("may not verify", self.go("rt"))
        self.assertIn("not the current head", self.go("qaer", sha="1234567"))


class VerifiedRerun(unittest.TestCase):
    """Only an accepted /verified of the current head may re-run the pull request check (#210)."""

    def rerun(self, actor, text, author="gpuer"):
        st = state(issue(1, labels=HELD))
        st["pr"] = pr(author=author)
        return rerun_check(decide({"kind": "pr", "number": 10, "actor": actor, "body": text}, st, CFG))

    def test_accepted_current_head_reruns(self):
        self.assertTrue(self.rerun("qaer", "/verified " + SHA[:12]))
        self.assertTrue(self.rerun("maint", "/verified " + SHA + "\nran the acceptance command"))

    def test_ordinary_comments_do_not_rerun(self):
        for text in ("looks good", "LGTM /verified " + SHA, "", "thanks\n/verified " + SHA[:12]):
            self.assertFalse(self.rerun("qaer", text), text)

    def test_malformed_do_not_rerun(self):
        for text in ("/verified", "/verified abc", "/verifiedaaaaaaa", "/verified " + SHA[:6], "/Verified " + SHA[:12]):
            self.assertFalse(self.rerun("qaer", text), text)

    def test_stale_and_unknown_heads_do_not_rerun(self):
        self.assertFalse(self.rerun("qaer", "/verified " + "b" * 12))
        self.assertFalse(self.rerun("qaer", "/verified " + SHA[:11] + "b"))

    def test_unauthorized_do_not_rerun(self):
        self.assertFalse(self.rerun("gpuer", "/verified " + SHA[:12]))                 # author
        self.assertFalse(self.rerun("gpuer", "/verified " + SHA[:12], author="maint"))  # task holder
        self.assertFalse(self.rerun("rt", "/verified " + SHA[:12]))                    # not a verifier
        self.assertFalse(self.rerun("stranger", "/verified " + SHA[:12]))

    def test_rerun_agrees_with_the_check(self):
        # Whatever re-runs the check must also be counted by it, and the other way round.
        for actor in ("qaer", "maint", "gpuer", "rt"):
            for sha in (SHA[:12], SHA[:6], "b" * 12):
                counted = actor in run(pr(), [vc(actor, sha)])["verified_by"]
                self.assertEqual(self.rerun(actor, "/verified " + sha), counted, (actor, sha))


class Summary(unittest.TestCase):
    def test_shape_and_json(self):
        st = state(issue(1), issue(2, labels=HELD, area="cpu", prio="low"), issue(3, labels=("bug",)))
        st["prs"] = [pr()]
        s = json.loads(json.dumps(summary(st, "2026-01-10T12:00:00Z")))
        self.assertEqual(set(s), {"generated_at", "totals", "by_area", "tasks", "pull_requests"})
        self.assertEqual(s["totals"], {"ready": 1, "claimed": 1})
        self.assertEqual(s["by_area"]["gpu"]["high"]["ready"], [1])
        self.assertEqual(s["by_area"]["cpu"]["low"]["claimed"], [2])
        self.assertEqual([t["number"] for t in s["tasks"]], [1, 2])
        self.assertEqual(s["tasks"][1]["holder"], "gpuer")
        self.assertEqual(s["pull_requests"][0]["links"], [1])
