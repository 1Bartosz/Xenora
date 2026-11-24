# Agent task queue

Xenora's CPU and GPU work proceed in parallel, often by coding agents. This page
describes how work is posted, claimed and finished through GitHub issues so that
no person has to relay prompts between agents.

## Lifecycle

1. **Post.** Anyone opens an issue with the *Agent task* template. It names the
   owner area (`gpu`, `cpu` or `shared`), the files it may edit, the exact
   acceptance command and any issues it depends on. New tasks are labelled
   `agent:ready`.
2. **Claim.** An agent comments `/claim`. The *Agent task claims* workflow
   processes claims for one issue at a time, so the first claim wins. It
   replaces `agent:ready` with `agent:claimed` and `claimed-by:<login>`. A later
   claim is refused with a reply.
3. **Work.** The holder works on a branch named after the issue, for example
   `task/42-vop2-integer-semantics`, and edits only the listed files. A progress
   comment or a linked pull request keeps the claim alive.
4. **Finish.** The pull request says `Closes #42` and reports the acceptance
   command's output. Review and merge follow [CONTRIBUTING.md](../../CONTRIBUTING.md);
   an author never merges their own pull request.
5. **Lapse.** A claim with no activity for 24 hours returns to `agent:ready`.
   `/release` gives a task back early. `/blocked` pauses the lapse timer while a
   question is open; `/unblock` resumes it.

Only repository owners, members, collaborators and logins listed in
`.github/agent-claimers.txt` can claim. The workflow changes labels and posts
comments; it never runs text from an issue.

## Rules for posting tasks

- One task, one reviewable change. Split anything that needs more than one
  pull request.
- List every file the task may edit. Two open claims never list the same path;
  a task that needs a file another claim holds depends on that claim.
- `shared` tasks touch interfaces both owners use (guest memory, loader hooks,
  shared Cargo configuration). Agree the interface in the issue before anyone
  claims it.
- The acceptance command must fail before the change and pass after it.
- Use public sources only and follow the clean-room rules in
  [AGENTS.md](../../AGENTS.md).

## Agent loop

`scripts/agent-queue.sh` wraps the GitHub CLI for agents:

```sh
scripts/agent-queue.sh list gpu        # ready tasks in one area
scripts/agent-queue.sh next gpu        # first ready task whose dependencies are closed
scripts/agent-queue.sh claim 42        # waits up to two minutes for the workflow's decision
scripts/agent-queue.sh note 42 "Decoder tables regenerated; tests next."
scripts/agent-queue.sh held            # tasks you currently hold
scripts/agent-queue.sh release 42
scripts/agent-queue.sh new body.md "task: VOP2 integer semantics"
```

A scheduled agent can run this loop unattended: update `main`, finish or note
progress on held tasks, then claim the next ready task in its area. Agents post
new tasks for anything they find outside their area instead of editing it.
