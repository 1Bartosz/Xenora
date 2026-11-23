# Verified evidence

Scope is the stated candidate and fixture. AMD/MSL public semantics are
class B, measured M5 GPU behavior is class A, and observed LLVM tool
behavior is class E. CPU test runs establish software behavior for the
named inputs. They provide no Xbox hardware evidence. All listed runs
used native ARM64 macOS. Phase 02 and X1–X4 results refer to working-tree
snapshots on `4b9a4ce`, with separate artifact hashes and logs.

## Phase 01

| Gates | Command and observed acceptance | Provenance and raw evidence |
| --- | --- | --- |
| G1–G4 | `scripts/audit/native.sh` checked 15 executables and dependencies. Format, all-feature Clippy and workspace tests passed (54 default tests). `scripts/test-gpu.sh` passed all 12 validated M5 tests. `scripts/demo.sh` matched reference and hand-derived results on all three original fixtures. All exit 0. | `5c47900`, [commands](../../artifacts/assurance-5c47900/commands.log), [exits](../../artifacts/assurance-5c47900/exits.log). |
| G5–G6 | `cargo test --locked -p xenora-semantic-review` passed all 13 tests, exit 0. `cargo test --locked -p xenora-decoder --test gfx1030 -- --include-ignored` verified 303 comparisons across 15 forms, three malformed and six subset probes for Phase 01. R2 signed off. | `4b9a4ce` [oracle review](../../artifacts/mutation/source-4b9a4ce/g5-review.md), `5c47900` [LLVM log](../../artifacts/assurance-5c47900/llvm.log). |
| G7 | `CARGO_BUILD_JOBS=2 scripts/mutation.sh artifacts/mutation/source-4b9a4ce` recorded all 399 outcomes. 359/361 eligible behavioral kills (99.45%), 13 independently proven equivalents, 25 invalid builds. The timeout and reference SIGKILL remain in the denominator. Native semantic 12/12 and oracle-only 44/44 at code-identical `5c47900`. The source wrapper did not capture its exit; all outcomes and the completion transcript are intact. | R2 [source review](../../artifacts/mutation/source-4b9a4ce/g7-review.md), [campaign](../../artifacts/mutation/source-4b9a4ce/campaign.log), [mutation accounting](../testing/mutation.md). |
| G8, G13–G14 | `scripts/audit/claims.sh`, `python3 scripts/audit/status.py --check` and `scripts/audit/dependencies.sh` passed. R1 accepted 22 claims and scoped prose for Phase 01. The transitive audit covered five graphics crates. | [Claims review](../../artifacts/r1-phase01-g8-g12.md), [CPU boundary ADR](../adr/0004-cpu-boundary.md), [dependency output](../../artifacts/assurance-5c47900/dependencies.log). |

### G9 native parser fuzz campaigns

Run replay commands only for a justified changed candidate. Each target
completed without an unresolved crash or hang.

| Command | Executions / elapsed / exit | Candidate and evidence |
| --- | --- | --- |
| `scripts/fuzz.sh instruction_stream 600` | 123,522,954 / 601 s / 0 | `4b9a4ce`, [decoder log](../../artifacts/g9/4b9a4ce-20261123T041525Z/instruction_stream.log) |
| `scripts/fuzz.sh reference 600` | 240,077,291 / 601 s / 0 | `4b9a4ce`, [reference log](../../artifacts/g9/4b9a4ce-20261123T041525Z/reference.log) |
| `scripts/fuzz.sh fixture 600` | 244,422,168 / 601 s / 0 | `aff46b0`, identical Phase 01 fuzz inputs, [fixture log](../../artifacts/assurance-aff46b0/fuzz/fixture.log), SHA-256 `ea81caf9495b57fc617d6470d3cac0488e59d1d727f16bbe8eda470987333a28` |

The first two runs have [ARM64 binary provenance](../../artifacts/g9/4b9a4ce-20261123T041525Z/executable-provenance.log)
and [matching post-run executable and transcript hashes](../../artifacts/g9/4b9a4ce-20261123T041525Z/final.sha256).
Raw `.exit`, timing and environment files sit beside the logs. Do not
transfer G9 to changed Phase 02 sources.

## GPU slices

All four forms use only VGPR sources and destination under the bounded
Wave32 compute ABI. Propagation and lowering are in
[shader](../../crates/shader/src/lib.rs), encodings in
[ISA masks](../../crates/isa/src/rdna2.rs) and
[decoder](../../crates/decoder/src/lib.rs), CPU semantics in
[reference](../../crates/ref/src/lib.rs), independent expectations in
[validation](../../tests/validation/known_answers.rs) and
[LLVM comparator](../../crates/decoder/tests/gfx1030.rs).

| Form and original fixture | Implementation and evidence |
| --- | --- |
| `v_or_b32_e32`, [or.s](../../fixtures/rdna2/or.s) | Bitwise OR preserves unchecked input bits according to the mask. Independently authored CPU equivalents and LLVM encodings passed. The first checkpoint had 56 default tests and 312 LLVM comparisons across 16 forms. Validated M5 dispatch matched all 131 words. [OR log](../../artifacts/phase02-or/or-gpu.log). |
| `v_lshlrev_b32_e32`, `v_lshrrev_b32_e32`, [shifts.s](../../fixtures/rdna2/shifts.s) | Shift unsigned src1 by src0 modulo 32; independent tests cover count wrap, high bits and aliases. The combined checkpoint had 58 default tests, 324 LLVM comparisons across 18 forms, 5 malformed and 19 subset probes. All 12 native regressions and both OR/shift native tests passed, 131 exact words each. All recorded exits were 0, with validated M5 runs. [LLVM](../../artifacts/phase02-shifts/llvm.log) · [readback](../../artifacts/phase02-shifts/fixtures-gpu.log) · [native audit](../../artifacts/phase02-shifts/native.log). |
| `v_ashrrev_i32_e32`, [ashr.s](../../fixtures/rdna2/ashr.s) | Implemented signed src1 shift with five-bit count. ARM64 LLVM reproduced the fixture word `0x31ffff01` and binary byte for byte (class E). Reference and native tests exist but no corresponding Rust or GPU acceptance log was found. Independent acceptance remains open. [Reproduction and expected vectors](../../fixtures/rdna2/ashr.md) · [disassembly](../../artifacts/phase02-ashr/disassembly.log). |

The first two slices used separate independent fixture derivation.
Continue with [exact acceptance commands](NEXT_PRIORITIES.md#p0--finish-arithmetic-shift-acceptance).
Phase 02 changed-code fuzz/mutation, matched timing and final independent
candidate review remain pending.

## Guest experiments

Run from the repo root. For all X1–X4, use
`cargo test --locked --manifest-path experiments/x1/Cargo.toml --target aarch64-apple-darwin`.
Add `--features metal` for X3/X4 coverage. The runner uses the same manifest
and target with `cargo run`, followed by `-- x2`, `-- x3` or `-- x4`.
X1 uses no argument. Set `MTL_DEBUG_LAYER=1` and `MTL_SHADER_VALIDATION=1` for
GPU runs and use the [README](../../experiments/x1/README.md) for the full
environment, fixture regeneration and native checks.

| Milestone | Expected and observed result | Evidence / review |
| --- | --- | --- |
| X0 | Thin ARM64 driver, macOS system dependencies, measured Apple M5 GPU runs. | [Native driver audit](../../artifacts/x4-review/native.log). |
| X1 | Eight tests passed, including six faults. Original flat programs retired six/five instructions and stored 42 / 0xffffffff with exact RIP, flags and memory. ARM64 driver linked only against libSystem. | [Tests](../../artifacts/x1/tests.log), [trace](../../artifacts/x1/trace.log), [native](../../artifacts/x1/native.log). |
| X2 | Reviewed fixed-base PE loader reached 0x140001000. Three guest instructions invoked service 1, returned 42 from argument 41, and stored `2a 00 00 00 00 00 00 00`. Independent 16 tests passed, including 0, 255 and max argument rollover and fault preservation. | [Independent oracle](../../artifacts/x2-review/checks.md), [native trace](../../artifacts/x2-review/native-trace.log), [tests](../../artifacts/x2-review/tests.log). |
| X3 | Corrected PE-to-Metal chain independently accepted. 21 tests passed; invalid entry and altered trap reached no dispatch. Five validated fresh runs matched `00ff00ff 12ff56ff ffffffff` and zero guest status. | [Review](../../artifacts/x3-pe.pQUnzE/review.md), [build/tests](../../artifacts/x3-pe.pQUnzE/review-build.log), [GPU](../../artifacts/x3-pe.pQUnzE/review-gpu.log). Earlier flat fixture alone did not pass acceptance. |
| X4 | Original PE draws for p=17 and p=93. Three fresh GPU processes each rendered both 8×8 RGBA8 textures, 384 total pixels all equal to `[x,y,p,255]`. 24 feature tests passed; invalid inputs blocked draw and an injected failure preserved pending status. Three recorded process exits were 0. | [Independent review](../../artifacts/x4-review/review.md), [oracle](../../artifacts/x4-review/oracle.log), [tests](../../artifacts/x4-review/tests.log), [native evidence](../../artifacts/x4-review/native.log). Actual GPU allocation or execution failure was not induced. |

## Retention

All linked raw logs under `artifacts/` are ignored and untracked.
Keep these directories in the same clone: `assurance-5c47900`,
`assurance-aff46b0/fuzz`, `mutation/source-4b9a4ce`, `g9`, `g11`,
`phase02-or`, `phase02-shifts`, `phase02-ashr`, `x1`, `x2`,
`x2-review`, `x3`, `x3-review`, `x3-pe.pQUnzE`, `x4`,
`x4-review`, `github-roadmap`, `github-cleanup` and `handoff`,
plus the standalone R1 logs. The G9 executables are under
`fuzz/target/aarch64-apple-darwin/release/`; their hashes survive in
the logs if binaries are rebuilt later. Transfer these records separately
when moving machines. Keep raw logs and binary outputs out of published
commits. The old GitHub synchronization scripts are historical evidence
and should not be reapplied to the consolidated backlog.
