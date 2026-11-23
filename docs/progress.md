# Engineering progress

**NOT RELEASE-READY.** The three-fixture demo checkpoint was pushed at `96cff7c66e55f1e4693ace19577c207795258d5c` and its [Native Rust checks](https://github.com/ranveersoni98/Xenora/actions/runs/45001680455) passed. See the [current assurance report](testing/phase01-assurance.md) for candidate evidence and the [historical engineering report](testing/phase01-checkpoint.md) for the initial checkpoint.

## Reading the progress graphic

- **Decoded forms** counts rows marked decoded in the [status registry](status.toml), out of all rows in that reviewed gfx1030 instruction inventory.
- **Accepted Metal forms** counts rows with accepted native fixture readback, over the same inventory. See the [form-by-form results](shaders/instruction-coverage.md) and [implementation checkpoint](testing/phase-02-baseline.md#implementation-checkpoint) for class A evidence from Apple M5. Arithmetic shift has its own [fixture record](../fixtures/rdna2/ashr.md).
- **Accepted guest milestones** counts the recorded fixture demonstrations for native guest instruction execution (X1), executable loading (X2), guest-to-GPU command submission (X3), and guest-driven Metal rendering (X4), out of those four milestones. See the [guest acceptance record](testing/phase-02-baseline.md#track-b--guest-execution).

The denominators describe this repository's fixture scope. Xbox compatibility remains unmeasured. After updating the registry with reviewed evidence, run `python3 scripts/audit/status.py` to regenerate the tables and [SVG](assets/xenora-progress.svg). CI uses the same command with `--check`.

<!-- status:features:start -->
| Feature | Target | Implementation | Correctness | Hardware | Evidence | Limits |
| --- | --- | --- | --- | --- | --- | --- |
| native-compute | arm64 macOS | implemented | initial-native-suite-passed | verified on Apple M5 | A | Trusted MSL with bounded word buffers and an original native offscreen rendering test on Apple M5. |
| synthetic-gfx1030 | gfx1030 Wave32 | three-fixture-demo-implemented | three-way-output-passed | verified on Apple M5 | A, B | Synthetic memory contract, statically resolved forward control flow, exact positive normal FP32 only. |
| xbox-execution | Xbox Series X\|S | unsupported | untested | unverified | U | Game execution, proprietary guest containers and full guest execution-mask divergence are unsupported. The console launch ABI is unknown. |
<!-- status:features:end -->

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

## Dual-track execution

The [dual-track baseline](testing/phase-02-baseline.md) records Phase 02
GPU prerequisites and the separate guest execution milestones.

## Remaining work

See the [assurance checkpoint](testing/phase01-assurance.md) for the original gate ledger and commands, and the [independent review](testing/review-log.md#phase-01-evidence-recheck-at-4b9a4ce) for later findings.

1. The independent reference (G5), LLVM decoder comparison (G6), and mutation testing (G7) passed at the Phase 01 candidate. The [integrated review](testing/mutation.md#integrated-source-campaign-at-4b9a4ce) confirmed 359 of 361 non-equivalent source mutants killed, all 44 independent-oracle kills and all 12 native semantic kills. The form-by-form validation report is in the [validation inventory](../tests/validation/README.md).
2. Parser fuzzing (G9) passed for the Phase 01 candidate. Native decoder and reference runs at `4b9a4ce` exited 0 after 123,522,954 and 240,077,291 executions respectively, each in 601 seconds. The fixture run at `aff46b0` exited 0 after 244,422,168 executions in 601 seconds. The fuzz source, dependencies, script and fixture inputs are identical between these commits. Logs and executable hashes are under `artifacts/g9/4b9a4ce-20261123T041525Z/`. Phase 02 source changes require their own assurance.
3. Finish benchmark corpus and protocol coverage before rerunning on AC and an idle system. Preserve the [exploratory battery and AC records](testing/benchmarks.md).
4. Close independent claims, status, history and review findings against the integrated candidate. Verify the [runtime redistribution blockers](licenses.md#runtime-redistribution-review) before closing the redistribution gate (G11).
