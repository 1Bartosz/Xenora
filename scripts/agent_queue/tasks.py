# SPDX-License-Identifier: GPL-3.0-or-later
"""Parse issue-form bodies into tasks and validate dependencies. Text is data only."""
import hashlib
import re
from datetime import datetime, timezone

from . import config as cfgmod

SECTION = re.compile(r"^###\s+(.+?)\s*$", re.M)


def sections(body):
    body = (body or "").replace("\r\n", "\n")
    marks = list(SECTION.finditer(body))
    out = {}
    for i, m in enumerate(marks):
        end = marks[i + 1].start() if i + 1 < len(marks) else len(body)
        text = body[m.end():end].strip()
        out[m.group(1).strip().lower()] = "" if text == "_No response_" else text
    return out


def _lines(text):
    res = []
    for line in text.splitlines():
        line = line.strip()
        if line.startswith(("- ", "* ")):
            line = line[2:].strip()
        line = line.strip("`").strip()
        if line:
            res.append(line)
    return res


def normalise_acceptance(text):
    lines = [l.strip() for l in (text or "").replace("\r\n", "\n").split("\n")]
    while lines and lines[0].startswith("```"):
        lines.pop(0)
    while lines and lines[-1].startswith("```"):
        lines.pop()
    lines = [" ".join(l.split()) for l in lines if l]
    return "\n".join(lines)


def acceptance_hash(text):
    return hashlib.sha256(normalise_acceptance(text).encode("utf-8")).hexdigest()


def label_names(issue):
    out = []
    for l in issue.get("labels") or []:
        out.append(l["name"] if isinstance(l, dict) else str(l))
    return out


def holder_of(labels):
    for l in labels:
        if l.startswith("claimed-by:"):
            return l[len("claimed-by:"):]
    return ""


def label_value(labels, prefix):
    for l in labels:
        if l.startswith(prefix):
            return l[len(prefix):]
    return ""


def parse_task(issue):
    """Build a plain dict task from a gh issue dict {number,title,body,state,labels}."""
    sec = sections(issue.get("body"))
    labels = label_names(issue)
    area = sec.get("area", "").strip().lower() or label_value(labels, "area:")
    prio = sec.get("priority", "").strip().lower() or label_value(labels, "priority:")
    return {
        "number": int(issue["number"]),
        "title": issue.get("title", ""),
        "state": str(issue.get("state", "OPEN")).upper(),
        "labels": labels,
        "area": area if area in cfgmod.AREAS else "",
        "priority": prio if prio in cfgmod.PRIORITIES else "medium",
        "files": _lines(sec.get("files", "")),
        "depends": sorted({int(n) for n in re.findall(r"#(\d+)", sec.get("depends on", ""))}),
        "acceptance": sec.get("acceptance command", ""),
        "interface": sec.get("interface", ""),
        "holder": holder_of(labels),
    }


def task_state(t):
    if t["state"] != "OPEN":
        return "closed"
    for name in ("invalid", "conflict", "blocked", "claimed", "ready"):
        if "agent:" + name in t["labels"]:
            return name
    return "untracked"


def parse_time(s):
    return datetime.strptime(s, "%Y-%m-%dT%H:%M:%SZ").replace(tzinfo=timezone.utc)


def find_cycles(tasks):
    """Cycles among open tasks. tasks: {number: task}. Returns list of number lists."""
    openset = {n for n, t in tasks.items() if t["state"] == "OPEN"}
    color, stack, cycles = {}, [], []

    def visit(n):
        color[n] = 1
        stack.append(n)
        for d in tasks[n]["depends"]:
            if d not in openset:
                continue
            if color.get(d) == 1:
                cycles.append(stack[stack.index(d):] + [d])
            elif d not in color:
                visit(d)
        stack.pop()
        color[n] = 2

    for n in sorted(openset):
        if n not in color:
            visit(n)
    return cycles


def dep_problems(task, tasks):
    """Dependency problems for one task: missing, open and cyclic dependencies."""
    res = {"missing": [], "open": [], "cycle": []}
    for d in task["depends"]:
        dep = tasks.get(d)
        if dep is None or dep["state"] == "MISSING":
            res["missing"].append(d)
        elif dep["state"] == "OPEN":
            res["open"].append(d)
    for cyc in find_cycles(tasks):
        if task["number"] in cyc:
            res["cycle"] = cyc
            break
    return res
