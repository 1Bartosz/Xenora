# Start here

Xenora is an original Rust research emulator for native Apple Silicon macOS.
Its graphics path translates a bounded desktop RDNA2/gfx1030 subset to MSL
and runs it through Apple's Metal API. An isolated ARM64 interpreter
experiment runs original x86-64 programs from flat and fixed-base PE32+
fixtures and can submit synthetic compute and render commands. The long-term
target is Xbox Series X|S software, with GTA VI's genuine intro/title screen
as a stretch goal. Commercial Xbox execution remains unsupported.

## Open the same clone

Use `/Users/ranveersoni/Developer/Xenora` for
[ranveersoni98/Xenora](https://github.com/ranveersoni98/Xenora). At handoff,
`main`, its upstream `origin/main`, and the GitHub default branch all
pointed at `4b9a4cecf35c8d4d90d8ecdb9c1510e7bd260102`. The index was empty
and there were zero commits ahead or behind. Phase 02 implementation,
fixtures, tests and policy changes are in the working tree. See the
[session record](SESSION_2026-11-23.md#repository-state) for the inventory
and transfer requirements. Keep the workspace and its ignored artifacts.

## Read in order

1. [Engineering rules](../../AGENTS.md), [interfaces](../interfaces.md) and
   [progress](../progress.md).
2. [Session record and architecture](SESSION_2026-11-23.md),
   [reproducible evidence](VERIFIED_EVIDENCE.md), and
   [priorities](NEXT_PRIORITIES.md).
3. The original Master Engineering Prompt V3 governs Phase 01 architecture,
   evidence and G1–G15. Continuation V4 is historical evidence. The Dual-Track
   Phase 02 + GTA VI prompt governs Phase 02 scheduling and P2-01–P2-10,
   with the policies below taking precedence over its earlier reuse
   language. Preserved originals and checksums are in
   `artifacts/handoff/specifications/` and
   `artifacts/handoff/specifications.sha256` in this clone. The
   [bootstrap prompt](NEXT_AGENT_PROMPT.md) works without chat attachments.

## Code and status

The main graphics code lives under `crates/{isa,decoder,shader,ref,metal,cli}`.
The isolated CPU and guest-driven GPU driver is under `experiments/x1`.
Independent tests live in `tests/validation`; original AMDGPU fixtures are in
`fixtures/rdna2`. X0 native host capability and X1–X4 synthetic execution are
verified. OR and both logical shifts passed LLVM, reference and Apple M5
readback. Arithmetic right shift has source and LLVM evidence. Its Rust and
Metal acceptance is the first task for the next session.

Phase 01 passed 11 of 15 gates. G10 controlled benchmarks, G11 redistribution,
G12 personal DCO attestations and G15 independent release acceptance remain
open. Phase 02 candidate-specific fuzz, mutation and matched performance
evidence also remain open. See the [GitHub backlog](SESSION_2026-11-23.md#github).

## Host and quick checks

The tested host is ARM64 Apple M5, macOS 27.3, Xcode 27.2, Rust 1.100.0,
Homebrew LLVM 23.1.6 and Python 3.11 or later. GPU tests need real Metal API
and shader validation. Use `CARGO_BUILD_JOBS=2` and run heavy tasks serially.
Start with `git status --short --branch`, `git diff --check`,
`cargo fmt --all -- --check`, `scripts/audit/claims.sh`, and
`python3 scripts/audit/status.py --check`. See [targeted replay](NEXT_PRIORITIES.md#p0--finish-arithmetic-shift-acceptance)
before any expensive build or run.

## Standing policies

Use native ARM64 and Metal, with no Rosetta, Wine, CrossOver, Windows
compatibility layer, OpenGL fallback, Vulkan runtime or MoltenVK. Use original
Rust and public specifications with independent expected results. Leonida,
KyTx and other emulators may inform test selection. Do not copy, translate,
port or import their code, tables, internal metadata or architecture.
Preserve GPL-3.0-or-later, accurate notices, personal DCO and project
test-material provenance. See [contribution policy](../../CONTRIBUTING.md).

User files are checked for technical safety and compatibility. Xenora
does not investigate origin, demand ownership, receipts or accounts, host
or fetch commercial files, or provide circumvention. Project contributions
and tests follow their [separate provenance rules](../legal.md).

Every spawned agent must have explicitly requested and confirmed **low**
reasoning. Use independent ownership and reviewers. Keep synthetic, host
hardware, upstream and Xbox claims separate. Do not push or publish without
authorization.
