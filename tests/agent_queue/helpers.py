# SPDX-License-Identifier: GPL-3.0-or-later
import json
import os
import sys

sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "..", "scripts"))

from agent_queue import config as cfgmod  # noqa: E402
from agent_queue.tasks import parse_time  # noqa: E402

ROOT = os.path.join(os.path.dirname(__file__), "..", "..")
CFG = cfgmod.parse(json.dumps({
    "lease_hours": 24, "upstream": "o/r", "maintainers": ["maint"],
    "contributors": {"maint": ["cpu", "shared", "qa"], "gpuer": ["gpu", "shared"],
                     "rt": ["runtime"], "qaer": ["qa"]},
}))
NOW = parse_time("2026-01-10T12:00:00Z")


def body(area="gpu", files="src/gpu/a.rs", deps="", acceptance="cargo test -p gpu", prio="high", interface=""):
    return ("### Area\n\n%s\n\n### Priority\n\n%s\n\n### Goal\n\ng\n\n### Files\n\n%s\n\n### Acceptance command\n\n%s\n\n"
            "### Depends on\n\n%s\n\n### Interface\n\n%s\n\n### Context\n\n_No response_\n") % (
        area, prio, files, acceptance, deps or "_No response_", interface or "_No response_")


def issue(n, labels=("agent:ready",), state="OPEN", title=None, **kw):
    return {"number": n, "title": title or "task %d" % n, "state": state,
            "labels": [{"name": l} for l in labels], "body": body(**kw)}


def comment(user, text, at="2026-01-10T10:00:00Z"):
    return {"user": user, "body": text, "created_at": at}


def state(*issues, **extra):
    s = {"issues": list(issues), "prs": [], "task_comments": {}}
    s.update(extra)
    return s


def event(actor, text, n=1, kind="issue"):
    return {"kind": kind, "number": n, "actor": actor, "body": text}
