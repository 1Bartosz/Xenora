# SPDX-License-Identifier: GPL-3.0-or-later
"""Decides every unanswered issue command, oldest first.

Claim runs share one concurrency group. GitHub keeps one run in progress and at most one pending in
a group, and a newly queued run cancels the pending one, so the comment that started a cancelled
run would never be decided. Each run therefore decides every recent command that has no bot reply
yet, in comment order, and applies each decision's labels to its in-memory state so later commands
see earlier grants. Every reply carries a marker naming the comment it answers."""
import copy
import re
from datetime import timedelta

from . import lease as L
from .decide import decide, parse_command
from .tasks import label_names, parse_time

DONE = re.compile(r"<!-- agent-queue:done comment=(\d+) -->")


def done_marker(comment_id):
    return "<!-- agent-queue:done comment=%d -->" % comment_id


def answered(comments, bots=L.BOT):
    """Ids of comments a bot reply has answered. Markers from anyone else do not count."""
    return {int(m) for c in comments if c["user"] in bots for m in DONE.findall(c["body"])}


def _apply(state, action):
    for i in state["issues"]:
        if int(i["number"]) == action["issue"]:
            names = [n for n in label_names(i) if n not in action["remove_labels"]]
            names += [n for n in action["add_labels"] if n not in names]
            i["labels"] = [{"name": n} for n in names]


def window_hours(cfg):
    """How far back `gather --recent-commands` reads: one lease to decide, one more to expire."""
    return 2 * cfg["lease_hours"]


def replay(state, cfg, now, bots=L.BOT):
    """Actions for every unanswered command in state["recent_comments"]: decided if created within
    the lease, else answered as expired so that no request goes silently unanswered."""
    state = copy.deepcopy(state)
    since = now - timedelta(hours=cfg["lease_hours"])
    comments = state.get("recent_comments", [])
    done = answered(comments, bots)
    out = []
    for c in sorted(comments, key=lambda c: (parse_time(c["created_at"]), c["id"])):
        if c.get("pull_request") or c["id"] in done or c["user"] in bots:
            continue
        done.add(c["id"])  # a comment listed twice is decided once
        if parse_time(c["created_at"]) < since:
            if parse_command(c["body"])[0]:
                out.append({"issue": c["issue"], "add_labels": [], "remove_labels": [], "comment":
                            "Not decided: no queue run answered this command within %s hours. Post it again if it still applies.\n%s"
                            % (cfg["lease_hours"], done_marker(c["id"]))})
            continue
        actions = decide({"kind": "issue", "number": c["issue"], "actor": c["user"], "body": c["body"]}, state, cfg)
        if not actions:
            continue
        actions[-1]["comment"] += "\n" + done_marker(c["id"])
        for a in actions:
            _apply(state, a)
        out.extend(actions)
    return out
