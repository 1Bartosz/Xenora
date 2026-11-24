# SPDX-License-Identifier: GPL-3.0-or-later
"""Comment command decisions. Returns actions as data; the caller applies them."""
import re

from . import config as cfgmod
from . import lease as L
from . import tasks as T

COMMANDS = ("/claim", "/release", "/reassign", "/progress", "/blocked", "/unblock",
            "/approve-acceptance", "/verified")


def quote(text):
    """User text echoed in a bot reply. Markers count only from the bot, so the reply must not be able
    to carry one the user wrote: angle brackets are escaped and code spans are closed."""
    return str(text).replace("&", "&amp;").replace("<", "&lt;").replace(">", "&gt;").replace("`", "'")


def parse_command(body):
    first = (body or "").strip().split("\n", 1)[0].strip()
    parts = first.split(None, 1)
    if not parts or parts[0] not in COMMANDS:
        return None, ""
    return parts[0], (parts[1].strip() if len(parts) > 1 else "")


FULL_SHA = re.compile(r"[0-9a-f]{40}")


def verified_sha(body):
    """The SHA of a well-formed `/verified <40-hex sha>` first line, else "". A short prefix is
    refused: a pushed commit with the same short prefix is cheap to make."""
    cmd, arg = parse_command(body)
    parts = arg.split()
    if cmd != "/verified" or len(parts) != 1 or not FULL_SHA.fullmatch(parts[0]):
        return ""
    return parts[0]


def _act(issue, add=(), remove=(), comment=""):
    return {"issue": issue, "add_labels": list(add), "remove_labels": list(remove), "comment": comment}


def _reply(issue, text):
    return [_act(issue, comment=text)]


def task_index(state):
    return {int(i["number"]): T.parse_task(i) for i in state.get("issues", [])}


def claim_problem(task, tasks, cfg, actor):
    if "agent:ready" not in task["labels"]:
        who = " (held by %s)" % task["holder"] if task["holder"] else ""
        return "not claimable: the issue is not labelled agent:ready%s." % who
    if not task["area"]:
        return "not claimable: the Area field is missing or not one of %s." % ", ".join(cfgmod.AREAS)
    if not cfgmod.may_work_on(cfg, actor, task["area"]):
        return "%s is not allowed to claim area:%s tasks (see .github/agent-queue.json)." % (actor, task["area"])
    if not task["files"]:
        return "not claimable: the Files field lists no paths."
    d = T.dep_problems(task, tasks)
    if d["missing"]:
        return "dependency issue(s) not found: %s." % ", ".join("#%d" % n for n in d["missing"])
    if d["cycle"]:
        return "dependency cycle: %s." % " -> ".join("#%d" % n for n in d["cycle"])
    if d["open"]:
        return "open dependencies: %s." % ", ".join("#%d" % n for n in d["open"])
    from .patterns import sets_overlap
    for other in sorted(tasks.values(), key=lambda t: t["number"]):
        if other["number"] == task["number"] or other["state"] != "OPEN":
            continue
        if "agent:claimed" not in other["labels"]:
            continue
        hit = sets_overlap(task["files"], other["files"])
        if hit:
            return "file overlap with claimed task #%d (`%s` vs `%s`)." % (other["number"], quote(hit[0]), quote(hit[1]))
    return ""


def decide(event, state, cfg):
    cmd, arg = parse_command(event.get("body"))
    if cmd is None:
        return []
    actor, n = event["actor"], int(event["number"])
    if cmd == "/verified":
        return _verified(event, state, cfg, arg)
    if event.get("kind") == "pr":
        return []
    tasks = task_index(state)
    task = tasks.get(n)
    if task is None or task["state"] != "OPEN":
        return []
    holder = task["holder"]
    maint = cfgmod.is_maintainer(cfg, actor)

    if cmd == "/claim":
        problem = claim_problem(task, tasks, cfg, actor)
        if problem:
            return _reply(n, "Claim refused for %s: %s" % (actor, problem))
        text = "Claimed by %s. The claim lapses %s hours after the latest grant, /progress comment or push to the linked pull request.\n%s" % (
            actor, cfg["lease_hours"], L.grant_marker(n, actor))
        return [_act(n, ["agent:claimed", "claimed-by:" + actor], ["agent:ready"], text)]

    if cmd == "/release":
        if "agent:claimed" not in task["labels"]:
            return _reply(n, "Nothing to release: the task is not claimed.")
        if actor != holder and not maint:
            return _reply(n, "Only the holder (%s) or a maintainer can release this claim." % holder)
        return [_act(n, ["agent:ready"], ["agent:claimed", "agent:blocked", "claimed-by:" + holder],
                     "Released by %s; the task is back in the queue." % actor)]

    if cmd == "/reassign":
        if not maint:
            return _reply(n, "Only a maintainer can reassign tasks.")
        target = arg.split()[0].lstrip("@") if arg else ""
        if not target:
            return _reply(n, "Usage: /reassign <login>")
        if not cfgmod.may_work_on(cfg, target, task["area"]):
            return _reply(n, "Reassign refused: %s is not allowed to work on area:%s." % (quote(target), task["area"]))
        if not (set(task["labels"]) & {"agent:claimed", "agent:ready"}):
            return _reply(n, "Reassign refused: the task is neither ready nor claimed.")
        # A reassignment creates a claim: the same dependency and file-overlap rules apply.
        problem = claim_problem(dict(task, labels=task["labels"] + ["agent:ready"]), tasks, cfg, target)
        if problem:
            return _reply(n, "Reassign refused: %s" % problem)
        remove = ["agent:ready"] + (["claimed-by:" + holder] if holder else [])
        return [_act(n, ["agent:claimed", "claimed-by:" + target], remove,
                     "Reassigned to %s by %s. New lease starts now.\n%s" % (target, actor, L.grant_marker(n, target)))]

    if cmd == "/progress":
        if "agent:claimed" not in task["labels"] or actor != holder:
            return _reply(n, "Only the holder of a claimed task can post progress.")
        if len(arg) < 3:
            return _reply(n, "Progress needs a short description of what changed, for example `/progress parser done, tests next`. Lease not renewed.")
        return _reply(n, "Lease renewed: %s more hours from now." % cfg["lease_hours"])

    if cmd == "/blocked":
        if actor != holder and not maint:
            return _reply(n, "Only the holder or a maintainer can mark a task blocked.")
        if not arg:
            return _reply(n, "Usage: /blocked <reason>")
        return [_act(n, ["agent:blocked"], [], "Blocked by %s: %s" % (actor, quote(arg)))]

    if cmd == "/unblock":
        if actor != holder and not maint:
            return _reply(n, "Only the holder or a maintainer can unblock a task.")
        return [_act(n, [], ["agent:blocked"], "Unblocked by %s." % actor)]

    if cmd == "/approve-acceptance":
        if not maint:
            return _reply(n, "Only a maintainer can approve an acceptance command.")
        if not T.normalise_acceptance(task["acceptance"]):
            return _reply(n, "No acceptance command to approve.")
        digest = T.acceptance_hash(task["acceptance"])
        return _reply(n, "Acceptance command approved by %s for the current text (sha256 %s). Editing the issue body invalidates this approval; nothing runs it automatically.\n%s" % (
            actor, digest[:12], L.acceptance_marker(digest)))
    return []


def _verified(event, state, cfg, arg):
    n, actor = int(event["number"]), event["actor"]
    if event.get("kind") != "pr":
        return []
    pr = state.get("pr") or {}
    sha = verified_sha(event.get("body"))
    head = pr.get("head_sha", "")
    if actor == pr.get("author"):
        return _reply(n, "Verification refused: the pull request author cannot verify it.")
    tasks = task_index(state)
    for issue in L.linked_issues(pr.get("body")):
        if issue in tasks and tasks[issue]["holder"] == actor:
            return _reply(n, "Verification refused: %s holds the linked task #%d." % (actor, issue))
    if not cfgmod.is_verifier(cfg, actor):
        return _reply(n, "%s may not verify: only maintainers and qa contributors can." % actor)
    if not sha or sha != head:
        return _reply(n, "Verification refused: `%s` is not the full current head %s." % (quote(sha) or "(none)", head))
    # Only an accepted verification of the current head re-runs the pull request check.
    return [dict(_act(n, comment="Verification of %s recorded for %s." % (head[:12], actor)), rerun_check=True)]


def rerun_check(actions):
    """True when the decided actions record an accepted verification of the current head."""
    return any(a.get("rerun_check") for a in actions)
