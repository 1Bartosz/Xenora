# SPDX-License-Identifier: GPL-3.0-or-later
"""Queue-wide views: machine-readable summary and the scheduled audit."""
from . import config as cfgmod
from . import lease as L
from . import tasks as T
from .decide import quote, task_index
from .patterns import sets_overlap


def summary(state, now_iso=""):
    tasks = task_index(state)
    by_area, rows = {}, []
    for n in sorted(tasks):
        t = tasks[n]
        if t["state"] != "OPEN" or not any(l.startswith("agent:") for l in t["labels"]):
            continue
        st = T.task_state(t)
        area = t["area"] or "unset"
        by_area.setdefault(area, {}).setdefault(t["priority"], {}).setdefault(st, []).append(n)
        rows.append({"number": n, "title": t["title"], "area": area, "priority": t["priority"],
                     "state": st, "holder": t["holder"], "depends": t["depends"], "files": t["files"]})
    counts = {}
    for r in rows:
        counts[r["state"]] = counts.get(r["state"], 0) + 1
    prs = [{"number": p["number"], "author": p["author"], "head_sha": p.get("head_sha", ""),
            "links": L.linked_issues(p.get("body")), "files": len(p.get("files", []))}
           for p in state.get("prs", [])]
    return {"generated_at": now_iso, "totals": counts, "by_area": by_area, "tasks": rows, "pull_requests": prs}


def audit(state, cfg, now):
    """Actions that lapse stale claims and mark invalid or conflicting tasks."""
    tasks = task_index(state)
    actions = []
    cycle_nodes = {n for cyc in T.find_cycles(tasks) for n in cyc}
    claimed = [t for t in tasks.values() if t["state"] == "OPEN" and "agent:claimed" in t["labels"]]
    for t in sorted(claimed, key=lambda x: x["number"]):
        comments = state.get("task_comments", {}).get(str(t["number"]), [])
        if "agent:blocked" in t["labels"]:
            continue
        if L.lapsed(t, comments, state.get("prs", []), cfg, now):
            actions.append({"issue": t["number"], "add_labels": ["agent:ready"],
                            "remove_labels": ["agent:claimed", "claimed-by:" + t["holder"]],
                            "comment": "Claim lapsed after %s hours without progress; the task is back in the queue." % cfg["lease_hours"]})
    lapsed_nums = {a["issue"] for a in actions}
    for i, a in enumerate(claimed):
        for b in claimed[i + 1:]:
            hit = sets_overlap(a["files"], b["files"])
            if hit:
                for x, y in ((a, b), (b, a)):
                    if "agent:conflict" not in x["labels"] and x["number"] not in lapsed_nums:
                        actions.append({"issue": x["number"], "add_labels": ["agent:conflict"], "remove_labels": [],
                                        "comment": "File overlap with claimed task #%d (`%s` vs `%s`). A maintainer must resolve it." % (y["number"], quote(hit[0]), quote(hit[1]))})
    for t in sorted(tasks.values(), key=lambda x: x["number"]):
        if t["state"] != "OPEN" or "agent:ready" not in t["labels"]:
            continue
        d = T.dep_problems(t, tasks)
        if d["missing"] or d["cycle"] or t["number"] in cycle_nodes:
            why = ("missing dependencies %s" % d["missing"]) if d["missing"] else "dependency cycle %s" % d["cycle"]
            actions.append({"issue": t["number"], "add_labels": ["agent:invalid"], "remove_labels": ["agent:ready"],
                            "comment": "Marked invalid: %s. Fix the Depends on field to return it to the queue." % why})
    for t in sorted(tasks.values(), key=lambda x: x["number"]):
        if t["state"] == "OPEN" and "agent:invalid" in t["labels"] and "agent:claimed" not in t["labels"]:
            d = T.dep_problems(t, tasks)
            if not d["missing"] and not d["cycle"]:
                actions.append({"issue": t["number"], "add_labels": ["agent:ready"], "remove_labels": ["agent:invalid"],
                                "comment": "Dependencies are valid again; the task is back in the queue."})
    return actions
