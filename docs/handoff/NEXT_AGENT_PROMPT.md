# Ready-to-paste next-thread prompt

Continue Xenora in the **same existing local workspace**
`/Users/ranveersoni/Developer/Xenora`.
Repository: https://github.com/ranveersoni98/Xenora.
Xenora is an original GPL-3.0-or-later Rust research emulator for
native Apple Silicon ARM64 macOS and native Metal. The strategic target
is Xbox Series X|S software execution, with GTA VI's genuine intro/title
screen a stretch goal. Current evidence is for original synthetic programs.

**THIS IS A CONTINUATION OF EXISTING VERIFIED WORK. DO NOT RESTART XENORA OR REIMPLEMENT COMPLETED X1–X4 MILESTONES.**

Before editing or launching agents, read `docs/handoff/START_HERE.md`,
`SESSION_2026-11-23.md`, `VERIFIED_EVIDENCE.md` and `NEXT_PRIORITIES.md`
in that directory, plus `AGENTS.md`, `docs/interfaces.md` and
`docs/progress.md`. The preserved authoritative V3 and dual-track prompts
and historical V4 are under `artifacts/handoff/specifications/`. Use the
handoff's current policy corrections over historical upstream-reuse
language. Existing G1–G15 and P2-01–P2-10 criteria remain binding.

Inspect `git status --short --branch`, `git rev-parse HEAD`,
`git branch -vv`, `git worktree list`, tracked and staged diffs, and
nonignored untracked files. The handoff base was
`4b9a4cecf35c8d4d90d8ecdb9c1510e7bd260102` on `main` tracking
`origin/main`, with significant unstaged and untracked work, including
the entire guest experiment. Check actual state. Preserve code, fixtures,
logs and hashes. Do not reset, clean, stash, rebase, rewrite history or
discard user edits. No push, publication, release, tag or remote changes
without authorization. Do not append false human DCO attestations or
use a human identity to certify work.

Baseline: Phase 01 passed 11 gates, including G9. G10 controlled
benchmarks, G11 redistribution, G12 human DCO attestations and G15
final independent candidate review remain open. These are release
requirements. X1 performed bounded x86-64 interpretation, X2 loaded
original fixed-base PE32+ and dispatched a synthetic service, X3
independently proved original PE-driven native compute, and X4
independently verified original guest-driven 8×8 RGBA8 Metal draw.
VGPR-only gfx1030 OR and both logical shifts passed LLVM, independent
CPU expectations and validated Metal readback. ASHR is implemented
with LLVM-only acceptance evidence at handoff.

**First task:** independently verify `v_ashrrev_i32_e32` in the existing
decoder, IR, reference and MSL implementation. Use the exact targeted
commands in `docs/handoff/NEXT_PRIORITIES.md` and fixture instructions
in `fixtures/rdna2/ashr.md`. Check signed and unsigned interpretations,
count wrap, aliasing and malformed or unsupported cases. Run Rust,
LLVM and real Metal checks and keep exact output and exit evidence.
Fix any actual failures with focused regressions, reconcile the fixture
and status integration, then proceed with integration assurance and
original Wave64/EXEC fixtures followed by bounded mask semantics. Grow
guest semantics and mappings incrementally. Defer full ARM64
recompilation until correctness and profiling justify it.

Maintain Xenora's independently authored Rust architecture, decoder,
typed IR, compiler and native Metal backend. Use public specifications
and original fixtures. Never copy, port, translate or import another
emulator's code, tables, internal metadata or architecture. Leonida
and KyTx are ecosystem references only. No Rosetta, Wine, CrossOver,
Windows compatibility layer, OpenGL fallback, Vulkan or MoltenVK runtime.
Use runtime capability detection and verify real native ARM64 linkage.
No restricted source, SDKs, keys, game distributions or circumvention.

User-provided game files get technical format, bounds, compatibility
and security checks. Do not request ownership receipts, accounts, or
acquisition history, or reject a technically compatible file solely
because its source is unknown. Project-run tests and contributions
follow their own provenance rules.

Every spawned implementation or review agent must explicitly use
**LOW reasoning**, verified from the resolved settings. If low cannot
be guaranteed, do not spawn. Use nonoverlapping ownership and
independent specification-derived expectations or review. Keep heavy
builds, GPU runs, fuzz and benchmarks serial on the fanless M5, normally
two Cargo jobs. Reuse valid evidence. Do not repeat completed Phase 01
fuzz or mutation campaigns or accepted guest GPU work without a
specific changed-code risk. Benchmarks need correct workloads and
documented power, thermal and idle conditions.

Report what changed, exact file paths, base SHA and working-tree
snapshot, named tests, counts, exits, failures, tool and device
conditions, native GPU results, independent review and remaining
blockers. Distinguish tests that ran from skips or pending checks.
Never claim commercial game execution, speedups or closed gates without
matching evidence. After orientation, implement and verify the first
eligible task instead of rewriting the roadmap.
