# Agent task queue

Work items are GitHub issues. Contributors and coding agents claim them with a comment, work in
their own branch, and open a pull request that is checked against the claim. The queue is
deterministic: the rules live in `scripts/agent_queue/` (standard-library Python, no network, no
model or external service), and GitHub Actions and the shell helpers only call it.

Status: implemented and unit-tested locally. **Not yet run on GitHub**: the workflows are untested
against the real API until the first live run.

## Areas

| Area | Owner | Scope |
| --- | --- | --- |
| `cpu` | maintainer | CPU translation, ARM64 dynamic binary translation, PE loading, guest startup, core guest memory |
| `gpu` | GPU contributor | RDNA2, shaders, graphics, Metal |
| `runtime` | runtime contributor | Win32, CRT, C++ runtime, import generation, OS services, platform abstractions |
| `shared` | agreed by the owners involved | Interfaces that cross subsystems |
| `qa` | any qa contributor | Independent verification, regression testing, security review |

Who may claim what is in `.github/agent-queue.json` (`maintainers`, `contributors`, `lease_hours`,
`upstream`). The file is plain JSON, so the notes live here: the maintainer must add the runtime
contributor's GitHub login under `contributors` (for example `"their-login": ["runtime", "qa"]`)
before that person can claim. Maintainers may claim any area; others only the areas listed for them.

## One-time label bootstrap (maintainer)

```sh
for l in agent:ready agent:claimed agent:blocked agent:invalid agent:conflict \
         area:cpu area:gpu area:runtime area:shared area:qa \
         priority:critical priority:high priority:medium priority:low; do
  gh label create "$l" --force --color BFD4F2
done
```

`claimed-by:<login>` labels are created by the workflow when a claim is granted.

## Lifecycle

1. Post a task with the *Agent task* issue form or `scripts/agent-queue.sh new body.md "task: title"`.
   Area, Priority, Files (exact paths, `dir/` prefixes, `*` and `**` globs), Acceptance command,
   Depends on and, for shared work, Interface (interface name and the version or upstream commit
   it targets).
2. `scripts/agent-queue.sh next <area>` picks the highest-priority task you may claim now.
3. `scripts/agent-queue.sh claim <n>` posts `/claim` and waits until the bot confirms: a grant
   comment carrying a marker, and the label `claimed-by:<you>`. **Do not edit anything before it
   prints "claimed".** Refusals name the reason: not `agent:ready`, area not allowed for you, open
   or missing dependency, dependency cycle, or file overlap with a claimed task (the issue is named).
   Claims are decided one at a time across the whole repository, so two overlapping claims cannot both win.
4. `scripts/agent-queue.sh start <n>` creates branch `task/<n>-<slug>` from `upstream/main` in a
   separate worktree `../xenora-task-<n>` (`--here` uses the current clean checkout instead).
5. Keep the lease alive with `scripts/agent-queue.sh progress <n> "what changed"`. A claim lapses
   `lease_hours` (24) after the latest of: the grant, your `/progress` comment, a push to your
   linked pull request. Other people's comments never renew it. A lapsed task returns to the queue.
6. Open the pull request with `Closes #<n>` and a DCO sign-off (`git commit -s`).
7. A different person (not the author, not the task holder) who is a maintainer or has `qa`
   comments `/verified <head sha>` after running the acceptance command. A new push changes the
   head and invalidates the verification.

## Commands (issue comments)

| Command | Who | Effect |
| --- | --- | --- |
| `/claim` | allowed for the area | Take a ready task |
| `/release` | holder or maintainer | Return the task to the queue |
| `/reassign <login>` | maintainer | Hand the task to someone allowed for its area; new lease |
| `/progress <text>` | holder | Renew the lease (text required) |
| `/blocked <reason>`, `/unblock` | holder or maintainer | Set or clear `agent:blocked` |
| `/approve-acceptance` | maintainer | Authorise the acceptance command as currently written |
| `/verified <sha>` (on a pull request) | maintainer or `qa`, not author, not holder | Independent verification of the current head |

Labels: `area:<x>`, `priority:<critical|high|medium|low>`, `agent:ready|claimed|blocked|invalid|conflict`,
`claimed-by:<login>`. An hourly audit lapses stale claims, marks tasks with missing dependencies or
cycles `agent:invalid` (and restores them when fixed), flags overlapping claims `agent:conflict`, and
writes a JSON summary to the run summary. `scripts/agent-queue.sh status --json` prints the same
machine-readable summary (counts and lists per area, priority and state).

## Pull request checks

`agent-pr-checks` fails a pull request when: it has no `Closes #N` for a claimed task; the task is held
by someone else; it changes files outside the task's patterns; it changes a file that another open
pull request also changes; the claim has lapsed without a push; or no eligible `/verified <current
head sha>` exists. After a `/verified` comment the check is re-run automatically.

## Forks and upstream sync

Work from a fork, but synchronise against the real upstream (`ranveersoni98/Xenora` in the
config), never only your fork's `origin/main`. `scripts/agent-sync.sh` adds the remote `upstream` if
missing (it never touches `origin`), fetches every 12 minutes (`--interval 10..15`, `--once` for one
pass) and, after each fetch, lists new upstream commits that touch files of tasks you hold. It never
merges by itself; `--merge` merges `upstream/main` into the current task branch only when the tree
is clean. It works from any worktree.

Interface changes during work, for example the maintainer's unpublished changes landing: put the
interface in a `shared` task whose Interface field names the version or commit other tasks target.
Tasks that depend on it list it in Depends on, so they cannot be claimed before it closes, and a
sync report that shows upstream touching your files is the signal to re-read the interface and merge.

## Security model

- Issue and comment text is data. It is parsed by fixed rules and never evaluated, never put into a
  shell command line, and workflow expressions never appear inside `run:` blocks (event data is read
  from the event file).
- Acceptance commands are never run automatically. `scripts/agent-queue.sh acceptance <n>` prints
  one only when a maintainer's `/approve-acceptance` marker (posted by the bot) matches a hash of
  the current text; editing the issue body invalidates the approval. Read the command before running it.
- Markers (grant, approval) count only when posted by the Actions bot, so they cannot be forged in a comment.
- Workflows declare empty top-level permissions and grant each job only what it needs; actions are
  pinned by commit SHA. Pull request checks run on the base branch (`pull_request_target`), never
  check out or run pull request code and read changed files through the API.
- Clean-room, DCO and native ARM64 rules in `AGENTS.md` and `CONTRIBUTING.md` apply unchanged.

## Tests

```sh
PYTHONPATH=scripts python3 -m unittest discover -s tests/agent_queue -v
sh -n scripts/agent-queue.sh scripts/agent-sync.sh
```

The tests cover command decisions, overlap, cycles, leases, acceptance hashing, pull request checks,
the summary, the CLI and a structural check of the workflow files. They do not exercise GitHub itself.
