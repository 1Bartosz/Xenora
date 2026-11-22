# Phase 01 engineering checkpoint

**NOT RELEASE-READY.** The evidence below was recorded on 2026-11-22 against the integrated local working tree based on `9ab5bf9`. The release brief requires evidence against one final candidate commit. Every formal gate therefore remains **NOT MET**. A passing local check is identified separately from outstanding work.

| Gate | Local evidence and remaining requirement |
|---|---|
| G1 | `scripts/audit/native.sh` passed. The host reported arm64 with translation off. All 11 Cargo executables or loadable dependencies checked were arm64 and their linkage contained no Vulkan or MoltenVK. Repeat against the candidate and include all candidate artifacts. Log `artifacts/phase01/native-audit.log`. |
| G2 | `cargo fmt --all -- --check`, `cargo clippy --locked --workspace --all-targets --all-features -- -D warnings` and `cargo test --locked --workspace` passed. 24 tests passed, with the LLVM oracle opted out of the default run and passed separately. No production C++ exists. Candidate rerun required. |
| G3 | `scripts/test-gpu.sh` passed 12 tests with zero failures or skips on Apple M5. The runtime reported Metal API and GPU validation enabled. Includes original offscreen vertex and fragment MSL with 48 checked RGBA8 pixels. Candidate rerun required. Log `artifacts/phase01/native-gpu-final.log`. |
| G4 | `scripts/demo.sh` passed all three original fixtures with both validation layers on. Exact readbacks are below. Confirm the command from the final candidate checkout. Log `artifacts/phase01/demo-final.log`. |
| G5 | R2 passed four independent CPU tests across widened integer known answers, both SCC outcomes, malformed branch rejection and exact FP32 boundaries. Full independent reference-interpreter assurance and candidate certification remain open. |
| G6 | LLVM 23.1.6 matched production decoding for 34 fixture instructions spanning 15 forms. Automatic comparison with the independent reference decoder is pending. Log `artifacts/phase01/llvm-oracle.log`. |
| G7 | `cargo mutants --workspace --list --json` restricted to the decoder, compiler and reference sources enumerated 373 source mutants. The source, semantic and oracle campaigns and survivor triage remain pending. No kill rate is available. |
| G8 | `scripts/audit/claims.sh` passed for 15 entries (A=4, B=2, D=6, E=2, U=1). The lint checks registered references and verification state. The audit of all hardware claims in prose remains open. |
| G9 | An earlier compiler campaign was interrupted after 74,020,192 reported executions. The latest decoder campaign was interrupted after 106,796,894 reported executions, about 212 seconds, exit 130. Neither log reports a crash before interruption. Latest compiler and reference campaigns were not started. The required 600 seconds per target remain pending. |
| G10 | Benchmark implementation and reports are deferred. The host reported battery power. No measurements are available. |
| G11 | Pinned runtime and fuzz inventories and applicable repository notices are present. The upstream objc2 notice raises an unresolved SDK-derived binding redistribution question. Final independent license approval remains open. |
| G12 | Independent initial history and local output inspection found no matching private key or token patterns. A previously unignored standalone validation build folder is now ignored. Full candidate and history audits remain open. |
| G13 | README and machine-readable status reflect the three supported synthetic fixtures and the native suite. Final independent consistency review remains open. |
| G14 | The CPU-boundary ADR exists and `scripts/audit/dependencies.sh` passed across five graphics crates. Candidate rerun required. Log `artifacts/phase01/dependencies.log`. |
| G15 | Both reviewers completed two checkpoint passes. Final candidate review and closure of all release findings remain open. |

## Native execution evidence

`scripts/demo.sh` ran at 13:36:03 local time on Apple M5 using native Rust 1.100.0, Xcode 27.2 and macOS 27.3. Both Metal validation banners appeared. The script exited successfully and printed disassembly, IR, MSL and comparisons. For each row the Metal readback, independent CPU result and hand-derived answers were equal.

| Fixture | Exact output words |
|---|---|
| `add` | `[305419896, 305419897, 305419895, 0]` |
| `bitwise-branch` | `[305419896, 305419897, 315315847, 318723824]` |
| `float-exact` | `[1091305472, 1091305472, 1092616192, 1092616192]` |

These fixtures use an explicit synthetic binding ABI, statically resolved forward scalar control flow and the verified exact positive normal FP32 domain. Console containers, guest graphics stages and execution-mask divergence remain unsupported. See [fixture hashes and derivations](../../fixtures/manifest.toml), [supported instruction forms](../shaders/instruction-coverage.md) and [subnormal risks](../risk-register.md).

Eight agents performed the requested research, implementation and review assignments in waves. The [R1 review log](review-log.md) and [R2 validation report](../../tests/validation/README.md) retain findings and commands. Runtime and fuzz lockfiles passed RustSec advisory checks in this session. The hosted workflow was created and has yet to run. The 89/92 historical laboratory count is tracked separately in the [lab intake](../research/host-lab.md).
