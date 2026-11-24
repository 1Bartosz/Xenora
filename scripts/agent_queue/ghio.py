# SPDX-License-Identifier: GPL-3.0-or-later
"""Thin gh wrapper. Arguments are always passed as lists, never through a shell."""
import json
import subprocess


def gh(*args):
    r = subprocess.run(["gh"] + list(args), capture_output=True, text=True)
    if r.returncode != 0:
        raise RuntimeError("gh %s failed: %s" % (" ".join(args[:2]), r.stderr.strip()[:300]))
    return r.stdout


def ndjson(out):
    return [json.loads(l) for l in out.splitlines() if l.strip()]


def issue(repo, n):
    try:
        return json.loads(gh("issue", "view", str(n), "-R", repo, "--json", "number,title,body,state,labels"))
    except RuntimeError:
        return {"number": n, "title": "", "body": "", "state": "MISSING", "labels": []}


def comments(repo, n):
    jq = '.[] | {user: .user.login, body: .body, created_at: .created_at}'
    return ndjson(gh("api", "repos/%s/issues/%d/comments" % (repo, n), "--paginate", "--jq", jq))


def recent_comments(repo, since):
    """Issue and pull request comments updated since `since` (UTC ISO time), oldest first."""
    jq = ('.[] | {id: .id, user: .user.login, body: .body, created_at: .created_at, '
          'issue: (.issue_url | split("/") | last | tonumber), pull_request: (.html_url | test("/pull/"))}')
    return ndjson(gh("api", "repos/%s/issues/comments?sort=created&direction=asc&per_page=100&since=%s" % (repo, since),
                     "--paginate", "--jq", jq))


def open_issues(repo):
    return json.loads(gh("issue", "list", "-R", repo, "--state", "open", "--limit", "500",
                         "--json", "number,title,body,state,labels"))


def pr_data(repo, n):
    p = json.loads(gh("pr", "view", str(n), "-R", repo, "--json", "number,author,body,headRefOid,state"))
    sha = p["headRefOid"]
    when = gh("api", "repos/%s/commits/%s" % (repo, sha), "--jq", ".commit.committer.date").strip()
    files = gh("api", "repos/%s/pulls/%d/files" % (repo, n), "--paginate", "--jq", ".[].filename").split()
    return {"number": p["number"], "author": p["author"]["login"], "body": p["body"] or "",
            "head_sha": sha, "head_pushed_at": when, "files": files, "state": p["state"]}


def open_prs(repo):
    nums = [p["number"] for p in json.loads(gh("pr", "list", "-R", repo, "--state", "open", "--limit", "100", "--json", "number"))]
    return [pr_data(repo, n) for n in nums]


def gather(repo, with_prs=True, with_comments=True, recent_since=""):
    """State for decide/audit/summary/replay: open tasks plus any dependency issues they name and,
    with `recent_since`, every comment since then."""
    from .tasks import parse_task
    issues = {i["number"]: i for i in open_issues(repo)}
    for i in list(issues.values()):
        for d in parse_task(i)["depends"]:
            if d not in issues:
                issues[d] = issue(repo, d)
    state = {"issues": list(issues.values())}
    if with_comments:
        state["task_comments"] = {str(n): comments(repo, n) for n, i in issues.items()
                                  if any((l["name"] if isinstance(l, dict) else l) == "agent:claimed" for l in i["labels"])}
    if with_prs:
        state["prs"] = open_prs(repo)
    if recent_since:
        state["recent_comments"] = recent_comments(repo, recent_since)
    return state


def apply_actions(repo, actions):
    for a in actions:
        n = str(a["issue"])
        for label in a["add_labels"]:
            gh("label", "create", label, "-R", repo, "--force", "--color", "BFD4F2")
        args = ["issue", "edit", n, "-R", repo]
        for l in a["add_labels"]:
            args += ["--add-label", l]
        for l in a["remove_labels"]:
            args += ["--remove-label", l]
        if len(args) > 4:
            gh(*args)
        if a["comment"]:
            gh("issue", "comment", n, "-R", repo, "--body", a["comment"])
