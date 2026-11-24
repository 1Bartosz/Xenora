#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
# Agent task queue helper. See docs/coordination/agent-tasks.md.
set -eu

usage() {
    cat >&2 <<'EOF'
usage: scripts/agent-queue.sh list [gpu|cpu|shared]   open tasks ready to claim, oldest first
       scripts/agent-queue.sh next <area>              first ready task in <area> whose dependencies are closed
       scripts/agent-queue.sh claim <issue>            request the claim (the workflow decides)
       scripts/agent-queue.sh held                     tasks claimed by the current GitHub user
       scripts/agent-queue.sh note <issue> <text>      progress note; keeps the claim alive
       scripts/agent-queue.sh release <issue>          give the task back
       scripts/agent-queue.sh new <file.md> <title>    open a task from a body that follows the template
EOF
    exit 2
}

[ $# -ge 1 ] || usage
command=$1
shift

# One line per ready task: number, area, dependency text, title ("|"-separated, oldest first).
ready() {
    gh issue list --state open --label agent:ready --limit 200 \
        --json number,title,body,createdAt \
        -q 'sort_by(.createdAt) | .[] | [.number,
            ((.body | capture("### Area\\s+(?<a>[a-z]+)").a) // ""),
            ((.body | capture("### Depends on\\s+(?<d>[^\\n]*)").d) // ""),
            .title] | map(tostring) | join("|")'
}

case "$command" in
    list)
        want=${1:-}
        ready | while IFS="|" read -r number area deps title; do
            [ -z "$want" ] || [ "$area" = "$want" ] || continue
            printf '#%s\t%s\t%s\n' "$number" "$area" "$title"
        done
        ;;
    next)
        [ $# -eq 1 ] || usage
        ready | while IFS="|" read -r number area deps title; do
            [ "$area" = "$1" ] || continue
            open=no
            for dep in $(printf '%s\n' "$deps" | grep -oE '#[0-9]+' | tr -d '#'); do
                [ "$(gh issue view "$dep" --json state -q .state)" = CLOSED ] || open=yes
            done
            [ "$open" = no ] || continue
            printf '%s\n' "$number"
            break
        done
        ;;
    claim)
        [ $# -eq 1 ] || usage
        gh issue comment "$1" --body /claim >/dev/null
        me=$(gh api user -q .login)
        for _ in 1 2 3 4 5 6 7 8 9 10 11 12; do
            sleep 10
            if gh issue view "$1" --json labels -q '.labels[].name' | grep -qx "claimed-by:$me"; then
                echo "claimed #$1"
                exit 0
            fi
        done
        echo "claim of #$1 not granted; it may be held by someone else" >&2
        exit 1
        ;;
    held)
        me=$(gh api user -q .login)
        gh issue list --state open --label "claimed-by:$me" --json number,title -q '.[] | "#\(.number)\t\(.title)"'
        ;;
    note)
        [ $# -eq 2 ] || usage
        gh issue comment "$1" --body "$2" >/dev/null
        ;;
    release)
        [ $# -eq 1 ] || usage
        gh issue comment "$1" --body /release >/dev/null
        ;;
    new)
        [ $# -eq 2 ] || usage
        gh issue create --label agent:ready --title "$2" --body-file "$1"
        ;;
    *)
        usage
        ;;
esac
