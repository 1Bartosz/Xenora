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
LLVM_BIN="$(brew --prefix llvm)/bin" cargo test --locked -p xenora-decoder --test gfx1030 -- --ignored --nocapture
scripts/test-gpu.sh
scripts/demo.sh
scripts/audit/native.sh
scripts/audit/claims.sh
scripts/audit/dependencies.sh
```

The independent CPU validation tests are part of the workspace. The native suite and optional LLVM oracle run separately so CPU-only CI can report them accurately. Final local logs are under ignored `artifacts/phase01/`. Keep the historical lab and candidate logs separate.

## Remaining work

1. R2 owns the automated three-way decoder check and broader independent interpreter validation. Start from [the independent test report](../tests/validation/README.md).
2. Run `scripts/fuzz.sh instruction_stream`, `scripts/fuzz.sh fixture` and `scripts/fuzz.sh reference` serially. Each defaults to 600 seconds. The interrupted corpus is retained under ignored `artifacts/fuzz/` and no campaign remains running.
3. R2 owns source, semantic and oracle mutation execution and survivor triage. Recreate the source population with `cargo mutants --workspace --list --json --file 'crates/decoder/src/**' --file 'crates/shader/src/**' --file 'crates/ref/src/**'` before executing it. The first inventory has 373 entries. Resolve any new tests and reassess the denominator before reporting a gate result.
4. Implement benchmark coverage across the correctness corpus and execute on AC power with the machine idle. The checkout has verified correctness results and no benchmark measurements.
5. R1 owns the license, history, claim coverage and status audits. Confirm attribution completeness and the Apple SDK terms for the proposed linked distributable using the [runtime redistribution review](licenses.md#runtime-redistribution-review). Repeat both reviewer passes against the final candidate commit.
