#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
# Agent task queue helper: a thin wrapper over `python3 -m agent_queue` and gh.
# Always talks to the upstream repository named in .github/agent-queue.json, not to a fork.
# See docs/coordination/agent-tasks.md. Issue text is data: nothing from it is ever executed.
set -eu

root=$(cd "$(dirname "$0")/.." && pwd)
PYTHONPATH="$root/scripts"
export PYTHONPATH
aq() { python3 -m agent_queue --config "$root/.github/agent-queue.json" "$@"; }
repo=$(aq config-get upstream)
tmp=$(mktemp -d "${TMPDIR:-/tmp}/agent-queue.XXXXXX")
trap 'rm -rf "$tmp"' EXIT INT TERM

usage() {
    cat >&2 <<'USAGE'
usage: scripts/agent-queue.sh list [area] [--json]    ready tasks, by priority then number
       scripts/agent-queue.sh next <area>              best claimable task for you in <area>
       scripts/agent-queue.sh claim <issue>            claim and wait for the confirmed grant
       scripts/agent-queue.sh start <issue> [--here]   branch task/<n>-<slug> from upstream/main in a new worktree
       scripts/agent-queue.sh progress <issue> <text>  progress note; renews the lease
       scripts/agent-queue.sh release <issue>          give the task back
       scripts/agent-queue.sh reassign <issue> <login> maintainers only
       scripts/agent-queue.sh acceptance <issue>       print the acceptance command only if a maintainer approved it
       scripts/agent-queue.sh status [--json]          queue summary
       scripts/agent-queue.sh new <body.md> <title>    open a task from a body that follows the template
USAGE
    exit 2
}

isnum() { case "$1" in ''|*[!0-9]*) echo "not an issue number: $1" >&2; exit 2 ;; esac; }
me() { gh api user -q .login; }
state() { aq gather --repo "$repo" --no-prs > "$tmp/state.json"; }
fetch() { aq fetch-issue --repo "$repo" --issue "$1" > "$tmp/issue.json"; }
post() { gh issue comment "$1" -R "$repo" --body "$2" >/dev/null; }

[ $# -ge 1 ] || usage
command=$1
shift

case "$command" in
    list)
        area=""
        json=""
        for a in "$@"; do
            case "$a" in --json) json=--json ;; *) area=$a ;; esac
        done
        state
        aq list --state "$tmp/state.json" --area "$area" $json
        ;;
    next)
        [ $# -eq 1 ] || usage
        state
        aq next --state "$tmp/state.json" --area "$1" --login "$(me)"
        ;;
    claim)
        [ $# -eq 1 ] || usage
        isnum "$1"
        login=$(me)
        fetch "$1"
        skip=$(python3 -c 'import json,sys; print(len(json.load(open(sys.argv[1]))["comments"]))' "$tmp/issue.json")
        post "$1" /claim
        i=0
        while [ "$i" -lt 18 ]; do
            sleep 10
            i=$((i + 1))
            fetch "$1"
            rc=0
            msg=$(aq claim-status --data "$tmp/issue.json" --login "$login" --skip "$skip") || rc=$?
            case "$rc" in
                0) echo "claimed #$1 as $login; you may start editing"; exit 0 ;;
                1) echo "$msg" >&2; exit 1 ;;
            esac
        done
        echo "claim of #$1 not confirmed within 3 minutes; do not start editing" >&2
        exit 1
        ;;
    start)
        [ $# -ge 1 ] || usage
        isnum "$1"
        n=$1
        here=no
        [ "${2:-}" != "--here" ] || here=yes
        login=$(me)
        fetch "$n"
        rc=0
        aq claim-status --data "$tmp/issue.json" --login "$login" >/dev/null || rc=$?
        [ "$rc" -eq 0 ] || { echo "no confirmed claim of #$n for $login: run claim first" >&2; exit 1; }
        slug=$(python3 -c 'import json,re,sys; print(re.sub(r"[^a-z0-9]+","-",json.load(open(sys.argv[1]))["issues"][0]["title"].lower().replace("task:","")).strip("-")[:40])' "$tmp/issue.json")
        branch="task/$n${slug:+-$slug}"
        sh "$root/scripts/agent-sync.sh" --setup
        git -C "$root" fetch upstream
        if [ "$here" = yes ]; then
            [ -z "$(git -C "$root" status --porcelain)" ] || { echo "working tree not clean" >&2; exit 1; }
            git -C "$root" switch -c "$branch" upstream/main
            echo "on $branch (from upstream/main)"
        else
            dir="$root/../xenora-task-$n"
            git -C "$root" worktree add -b "$branch" "$dir" upstream/main
            echo "worktree $dir on $branch (from upstream/main)"
        fi
        ;;
    progress)
        [ $# -ge 2 ] || usage
        isnum "$1"
        n=$1
        shift
        post "$n" "/progress $*"
        ;;
    release)
        [ $# -eq 1 ] || usage
        isnum "$1"
        post "$1" /release
        ;;
    reassign)
        [ $# -eq 2 ] || usage
        isnum "$1"
        post "$1" "/reassign $2"
        ;;
    acceptance)
        [ $# -eq 1 ] || usage
        isnum "$1"
        fetch "$1"
        python3 - "$tmp/issue.json" "$tmp/i.json" "$tmp/c.json" <<'PY'
import json, sys
d = json.load(open(sys.argv[1]))
json.dump(d["issues"][0], open(sys.argv[2], "w"))
json.dump(d["comments"], open(sys.argv[3], "w"))
PY
        aq acceptance --issue "$tmp/i.json" --comments "$tmp/c.json"
        ;;
    status)
        state
        if [ "${1:-}" = "--json" ]; then
            aq summary --state "$tmp/state.json"
        else
            aq summary --state "$tmp/state.json" --text
        fi
        ;;
    new)
        [ $# -eq 2 ] || usage
        labels=$(aq check-body --file "$1")
        for l in agent:ready $labels; do
            gh label create "$l" -R "$repo" --force --color BFD4F2 >/dev/null
        done
        gh issue create -R "$repo" --label agent:ready --label "$(printf '%s\n' "$labels" | sed -n 1p)" \
            --label "$(printf '%s\n' "$labels" | sed -n 2p)" --title "$2" --body-file "$1"
        ;;
    *)
        usage
        ;;
esac
