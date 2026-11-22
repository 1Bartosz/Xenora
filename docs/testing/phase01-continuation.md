# Xenora Phase 01 continuation for Sol

Continue Phase 01 only in `~/Developer/Xenora`. Read the current AGENTS.md,
the Master Engineering Prompt V3 including section 40, and the
[assurance record](phase01-assurance.md). Preserve local commits and ignored
evidence. Do not push, publish, rewrite history, or start Phase 02.

**NOT RELEASE-READY.** Latest validated implementation candidate
`5c479005b0e0631a608a369b9e41327da61bb61e` was clean on main, four commits
ahead of origin/main before this documentation handoff. Run
`git status --short --branch` and `git rev-parse HEAD` first.

| Gate | Status | Evidence or blocker |
| --- | --- | --- |
| G1 | PASS | Native audit at aff46b0 checked 15 ARM64 workspace binaries/dependencies, no translation or Vulkan/MoltenVK. Exact integrated rerun in `artifacts/assurance-5c47900/native.log` when available. |
| G2 | PASS | Integrating 5c47900 passed format, all-target/all-feature Clippy with warnings denied, 54 default tests, two LLVM tests opt-in. Logs under `artifacts/r2-review-aff46b0/integrated-*` and exact candidate rerun under `artifacts/assurance-5c47900/`. |
| G3 | PASS | On aff46b0, Apple M5 passed 12 native tests with zero skips and both validation layers active. `artifacts/assurance-aff46b0/gpu.log`. |
| G4 | PASS | All three unchanged original shaders matched CPU reference and known answers on aff46b0. `artifacts/assurance-aff46b0/demo.log`. |
| G5 | BLOCKED | Integrated second-oracle suite passed 13 tests. R1 independently verified the integer FP32 proof and 20 rational boundary expectations. The latest locked candidate and oracle-only checks passed. Record final R2 sign-off. |
| G6 | PASS | R2 signed off on 303 comparisons across 15 forms, three malformed and six subset probes using Homebrew LLVM 23.1.6. Oracle inputs unchanged in 5c47900. |
| G7 | BLOCKED | Full source result applies only to aff46b0. The latest independent oracle-only rerun killed 44/44. The full source campaign on the integrated candidate remains pending. |
| G8 | BLOCKED | Claims lint passed with 20 entries, A4/B6/D7/E2/U1. Register AMD branch displacement and correct interpreter evidence scope, then finish independent prose audit. |
| G9 | BLOCKED | Historical three campaigns totaled 435,803,394 executions in 1,803 seconds. Refreshed fixture passed 244,422,168 executions in 601 seconds. Decoder/reference historical logs lack full candidate provenance. |
| G10 | BLOCKED | Exploratory AC run used cc0daa7, two warmups, 21 reps per size, release and validation off. Idle state unestablished. Complete corpus, stage and cold/warm timings, tool/capability metadata and variability protocol. |
| G11 | BLOCKED | SDK-derived binding redistribution and pinned attribution unresolved. Preserve objc2 and notices. No licensing clearance or binary publication. |
| G12 | BLOCKED | Strict fsck and scoped secret, path and build-artifact scans passed. Post-policy DCO attestations remain outstanding. See latest `hygiene.log`. |
| G13 | PASS | R1 passed generated status and claims consistency checks at 5c47900. CI invokes the check. |
| G14 | PASS | CPU boundary ADR and transitive check across five graphics crates passed. |
| G15 | BLOCKED | R1 and R2 reported independently. Open source revalidation, claims, benchmark and licensing findings prevent final closure. |

## G7 evidence

The independently audited aff46b0 campaign generated 399 mutations. Raw
counts were 360 caught, 13 missed, 25 unviable and one timeout. The 13
misses are proven equivalents and the 25 unviables are rustc failures.
One raw caught mutant was a reference non-progress SIGKILL without a
behavioral assertion. Count 359/361 = **99.45%**, retaining both
non-progress cases in the denominator. Read
[the full report and per-mutant proofs](mutation.md). Raw ledger and
outcomes are in `artifacts/mutation/source-aff46b0/`.

The 12 semantic GPU mutations returned different outputs by design.
Every unmodified shader matched its independent known answers. The
aff46b0 oracle-only campaign caught 37 of 44. New independent witnesses
for all seven gaps are committed at 5c47900. Do not transfer the old
source score to the changed test suite.

## Resume in this order

1. Inspect `artifacts/assurance-aff46b0/fuzz/{environment.log,fixture.log,fixture.exit,fixture.finished}`.
   The 600-second fixture job started on clean aff46b0. The relevant
   source, dependency, fixture and fuzz inputs are identical at 5c47900
   as checked in `artifacts/assurance-5c47900/fuzz-input-diff.log`.
   Preserve raw historical logs under `artifacts/g9/`.
2. Inspect `artifacts/assurance-5c47900/{commands,exits,oracle}.log`
   and `artifacts/mutation/oracle-5c47900/mutants.out/`. The oracle-only
   campaign and exact-candidate checks were queued after fuzz. If already
   complete, use the logs. Preserve both candidate epochs.
3. Run the complete source campaign once against the integrated candidate.
   Independently inspect every survivor, timeout, invalid and non-assertion
   failure. Close G5/G7 only after appropriate candidate review.
4. Finish G9 provenance, G8 prose audit, G10 protocol and G15 sign-off.
   Keep G11 blocked until actual resolution.

```sh
cd ~/Developer/Xenora
git status --short --branch
git rev-parse HEAD
export CARGO_BUILD_JOBS=2
export LLVM_BIN="$(brew --prefix llvm)/bin"
export PATH="$LLVM_BIN:$PATH"
candidate="$(git rev-parse --short HEAD)"
cargo test --locked -p xenora-semantic-review
cargo mutants --file crates/ref/src/lib.rs \
  --re ' in (vector_alu|execute_lane)$|replace (vector_alu|execute_lane) ->' \
  --test-package xenora-semantic-review --jobs 2 --timeout 60 \
  --output "artifacts/mutation/oracle-$candidate"
scripts/mutation.sh "artifacts/mutation/source-$candidate"
```

Perform needed campaigns serially. If affected fixture evidence is
missing, use `scripts/fuzz.sh fixture 600`. For exact candidate provenance
of the other two targets, use `scripts/fuzz.sh instruction_stream 600`
then `scripts/fuzz.sh reference 600`, capturing individual exit codes.
For remaining short integration and native checks use the commands in
[the assurance checkpoint](phase01-assurance.md).

The AC report is
[run 1795352394](../../benchmarks/results/apple-m5/run-1795352394/report.md).
Binding rights and notice blockers are in [the license inventory](../licenses.md).
Only original bounded synthetic gfx1030 compute fixtures are supported.
Game execution, proprietary guest containers, full guest mask divergence
and a guest CPU runtime remain unsupported.

## Completed fixture refresh

The affected campaign finished with exit 0 after 601 seconds and
244,422,168 executions. No crash, hang or sanitizer failure was recorded.
It used native nightly Rust and cargo-fuzz 0.13.2. The transcript SHA-256
is `ea81caf9495b57fc617d6470d3cac0488e59d1d727f16bbe8eda470987333a28`. It ran at aff46b0 with the same fuzz inputs as 5c47900.

## Exact integrated candidate results

All commands in `artifacts/assurance-5c47900/commands.log` completed with
exit 0. This includes 54 workspace tests, the separate LLVM oracle, 12
validated GPU tests, three end-to-end demos, 12 semantic mutant kills,
native and dependency audits, and claims and generated status checks.
The oracle-only campaign killed 44/44 in 23 seconds. Each outcome has
a named Rust test failure. All seven prior oracle survivors were killed.
This closes the observed oracle test gaps. The full source score stays
attached to aff46b0 and G7 remains open for the integrated candidate.
