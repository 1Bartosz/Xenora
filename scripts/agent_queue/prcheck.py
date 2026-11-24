# SPDX-License-Identifier: GPL-3.0-or-later
"""Pull request checks computed from data only. PR code is never fetched or run."""
from . import config as cfgmod
from . import lease as L
from .patterns import path_matches, patterns_overlap
from .decide import task_index


def verified_by(pr, comments, tasks, cfg):
    """Eligible verifier logins whose /verified comment names the current head SHA."""
    head, out = pr.get("head_sha", ""), []
    holders = {tasks[i]["holder"] for i in L.linked_issues(pr.get("body")) if i in tasks}
    for c in comments:
        parts = c["body"].strip().split("\n", 1)[0].split()
        if len(parts) < 2 or parts[0] != "/verified" or len(parts[1]) < 7:
            continue
        who = c["user"]
        if (head.startswith(parts[1]) and who != pr.get("author") and who not in holders
                and cfgmod.is_verifier(cfg, who) and who not in out):
            out.append(who)
    return out


def check_pr(state, cfg, now):
    pr, comments = state["pr"], state.get("pr_comments", [])
    tasks = task_index(state)
    problems = []
    links = [i for i in L.linked_issues(pr.get("body")) if i in tasks]
    if not links:
        problems.append("no linked task: the pull request body must contain `Closes #N` for a claimed task")
    for n in links:
        t = tasks[n]
        if "agent:claimed" not in t["labels"]:
            problems.append("task #%d is not claimed" % n)
            continue
        if t["holder"] != pr["author"]:
            problems.append("task #%d is held by %s, not the pull request author" % (n, t["holder"] or "nobody"))
        outside = [f for f in pr.get("files", []) if not any(path_matches(f, p) for p in t["files"])]
        if outside:
            problems.append("files outside task #%d ownership: %s" % (n, ", ".join(outside[:10])))
        if L.lapsed(t, state.get("task_comments", {}).get(str(n), []), [pr], cfg, now):
            problems.append("task #%d claim has lapsed and the head has not been pushed within %s hours" % (n, cfg["lease_hours"]))
    for other in state.get("prs", []):
        if other["number"] == pr["number"]:
            continue
        shared = sorted(set(pr.get("files", [])) & set(other.get("files", [])))
        if shared:
            problems.append("overlaps open PR #%d on: %s" % (other["number"], ", ".join(shared[:10])))
    ok_by = verified_by(pr, comments, tasks, cfg)
    if not ok_by:
        problems.append("independent verification missing: an eligible verifier must comment `/verified %s`" % pr.get("head_sha", "")[:12])
    return {"pr": pr["number"], "ok": not problems, "problems": problems, "verified_by": ok_by}
