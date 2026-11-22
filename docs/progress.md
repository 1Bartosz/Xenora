# Phase 01 checkpoint

**NOT RELEASE-READY.** The three-fixture demo checkpoint was pushed at `96cff7c66e55f1e4693ace19577c207795258d5c` and its [Native Rust checks](https://github.com/ranveersoni98/Xenora/actions/runs/45001680455) passed. See the [engineering report](testing/phase01-checkpoint.md) for gate status and subsequent assurance evidence.

## Reproduce

Run on native Apple Silicon with Rust 1.100.0, Xcode 27.2, Python 3.11 or later and LLVM 23.1.6. The verified host was Apple M5 with macOS 27.3. Metal's safe and precise compile options must be available. Use up to four Cargo jobs total across at most two builds, one GPU workload and one fuzz or mutation campaign at a time. The final checkpoint used two Cargo jobs.

```sh
export CARGO_BUILD_JOBS=2
scripts/bootstrap.sh
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
cargo test --locked --workspace
scripts/assemble-fixture.sh --check
LLVM_BIN="$(brew --prefix llvm)/bin" cargo test --locked -p xenora-decoder --test gfx1030 -- --include-ignored --nocapture
scripts/test-gpu.sh
scripts/demo.sh
scripts/audit/native.sh
scripts/audit/claims.sh
scripts/audit/dependencies.sh
```

The independent CPU validation tests are part of the workspace. The native suite and optional LLVM oracle run separately so CPU-only CI can report them accurately. Final local logs are under ignored `artifacts/phase01/`. Keep the historical lab and candidate logs separate.

## Remaining work

1. G7 is the immediate priority. The historical source campaign at `a34dadc` reported 387 mutants. The targeted rerun and accounting rules are in [mutation testing](testing/mutation.md). Run the complete source campaign after survivor triage, then complete the semantic and oracle campaigns with R2.
2. G5–G6 require independent R2 validation against the candidate. The automated three-way comparison covers all 15 decoded forms. Reproduce with the LLVM command above.
3. G9 has three successful historical 601-second transcripts in `artifacts/g9/`. Record candidate scope, nightly version and individual process exit codes on the confirmation run. Run `scripts/fuzz.sh instruction_stream`, `scripts/fuzz.sh fixture` and `scripts/fuzz.sh reference` serially.
4. G10 has a tracked [battery run](../benchmarks/results/apple-m5/run-1795338391/report.md). AC measurements and full corpus and protocol coverage remain open.
5. G11 remains open for objc2 SDK-derived binding redistribution and the pinned license inventory audit. See the [runtime redistribution review](licenses.md#runtime-redistribution-review). R1 also owns the claims, history and status audits. Both reviewers must review the final candidate.
