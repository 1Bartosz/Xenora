# Phase 01 checkpoint

**NOT RELEASE-READY.** The three-fixture demo checkpoint was pushed at `96cff7c66e55f1e4693ace19577c207795258d5c` and its [Native Rust checks](https://github.com/ranveersoni98/Xenora/actions/runs/45001680455) passed. See the [current assurance report](testing/phase01-assurance.md) for candidate evidence and the [historical engineering report](testing/phase01-checkpoint.md) for the initial checkpoint.

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

Use the [copy-ready continuation handoff](testing/phase01-continuation.md) for exact evidence, gate statuses and commands.

1. Finish G7 independent oracle assurance. The complete source campaign at `aff46b0` killed 359 of 361 non-equivalent eligible mutants. The native semantic campaign killed all 12. The separate 44-case oracle campaign exposed seven independent-test gaps. R2’s witness tests are integrated. Rerun the [mutation commands](testing/mutation.md) against the resulting candidate.
2. Preserve the G6 R2 sign-off. The G5 form-by-form report is in `tests/validation/README.md`. Confirm independent-oracle sensitivity on the integrated candidate. Run `cargo test --locked -p xenora-semantic-review` before candidate regression and mutation runs.
3. The affected fixture run passed 244,422,168 executions in 601 seconds. Confirm the remaining candidate provenance. `CARGO_BUILD_JOBS=2 scripts/fuzz.sh fixture 600`. Decoder and reference historical transcripts are under `artifacts/g9/`.
4. Finish benchmark corpus and protocol coverage before rerunning on AC and an idle system. Preserve the [exploratory battery and AC records](testing/benchmarks.md).
5. Close independent claims, status, history and review findings against the integrated candidate. Verify the [runtime redistribution blockers](licenses.md#runtime-redistribution-review) before claiming G11.
