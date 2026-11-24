# SPDX-License-Identifier: GPL-3.0-or-later
"""Claim leases. Only the holder's own activity renews one; markers count only from the bot."""
import re
from datetime import timedelta

from .tasks import parse_time

BOT = ["github-actions[bot]"]
GRANT = re.compile(r"<!-- agent-queue:grant issue=(\d+) login=([A-Za-z0-9-]+) -->")
ACCEPT = re.compile(r"<!-- agent-queue:acceptance sha256=([0-9a-f]{64}) -->")


def grant_marker(issue, login):
    return "<!-- agent-queue:grant issue=%d login=%s -->" % (issue, login)


def acceptance_marker(digest):
    return "<!-- agent-queue:acceptance sha256=%s -->" % digest


def latest_grant(comments, issue, login, bots=BOT):
    best = None
    for c in comments:
        if c["user"] not in bots:
            continue
        m = GRANT.search(c["body"])
        if m and int(m.group(1)) == issue and m.group(2) == login:
            t = parse_time(c["created_at"])
            best = t if best is None or t > best else best
    return best


def lease_start(task, comments, prs, bots=BOT):
    """Latest of grant, holder /progress comment, holder's linked PR push."""
    holder, n = task["holder"], task["number"]
    times = []
    g = latest_grant(comments, n, holder, bots)
    if g:
        times.append(g)
    for c in comments:
        if c["user"] == holder and c["body"].lstrip().startswith("/progress"):
            times.append(parse_time(c["created_at"]))
    for pr in prs:
        if pr["author"] == holder and n in linked_issues(pr["body"]) and pr.get("head_pushed_at"):
            times.append(parse_time(pr["head_pushed_at"]))
    return max(times) if times else None


def lease_end(task, comments, prs, cfg, bots=BOT):
    start = lease_start(task, comments, prs, bots)
    return None if start is None else start + timedelta(hours=cfg["lease_hours"])


def lapsed(task, comments, prs, cfg, now, bots=BOT):
    end = lease_end(task, comments, prs, cfg, bots)
    return end is not None and now > end


def linked_issues(body):
    return [int(n) for n in re.findall(r"(?i)\b(?:closes|fixes|resolves)\s+#(\d+)", body or "")]


def approved_hash(comments, bots=BOT):
    best, best_t = None, None
    for c in comments:
        if c["user"] not in bots:
            continue
        m = ACCEPT.search(c["body"])
        if m:
            t = parse_time(c["created_at"])
            if best_t is None or t >= best_t:
                best, best_t = m.group(1), t
    return best
