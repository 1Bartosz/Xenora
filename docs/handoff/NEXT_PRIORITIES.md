# Next priorities

## P0 — Finish arithmetic shift acceptance

Begin with [#19](https://github.com/ranveersoni98/Xenora/issues/19).
The working-tree implementation is VGPR-only gfx1030 Wave32
`v_ashrrev_i32_e32`. An independently derived signed-shift oracle must
cover sign extension, a nonnegative source, counts modulo 32, aliasing,
register extrema and malformed or unsupported variants. Existing
LLVM evidence is valid for the fixture hash in
[ashr.md](../../fixtures/rdna2/ashr.md). Native Rust and Metal test cases
exist and have no recorded acceptance run. Verify their expected values
independently and use the exact commands below before updating status.

```sh
export CARGO_BUILD_JOBS=2
export LLVM_BIN="$(brew --prefix llvm)/bin"
cargo fmt --all -- --check
cargo test --locked -p xenora-semantic-review arithmetic_right_shift_preserves_sign_and_wraps_counts
cargo test --locked -p xenora-decoder --test gfx1030 -- --include-ignored --nocapture
MTL_DEBUG_LAYER=1 MTL_SHADER_VALIDATION=1 MTL_SHADER_VALIDATION_REPORT_TO_STDERR=1 \
  MTL_SHADER_VALIDATION_ABORT_ON_FAULT=1 timeout 30s \
  cargo test --locked -p xenora-semantic-review arithmetic_right_shift_native_metal_known_answers \
  -- --ignored --nocapture --test-threads=1
```

Capture separate exit codes, named test counts, input and output coverage,
fixture hashes, working-tree patch and device validation evidence. A
filtered run that discovers zero tests provides no acceptance evidence.
If a probe fails, keep it as a focused regression and rerun affected
checks. Check the fixture is represented in the standard assembly and
status workflow after acceptance. The current assembly script enumerates
the previous five fixtures and excludes ASHR.

Then reconcile integration: run workspace format, strict all-feature
Clippy, workspace tests, `scripts/test-gpu.sh`, `scripts/audit/native.sh`,
and the [isolated experiment's tests](../../experiments/x1/README.md)
with and without `metal` as needed by the affected paths. Preserve
independently accepted X1–X4 results and rerun their exact GPU paths
only for an affected integration. Use serial heavy jobs. Prepare
candidate-specific mutation and fuzz plans for changed paths
([#53](https://github.com/ranveersoni98/Xenora/issues/53)); use the
actual inherited acceptance criteria. A targeted campaign supplements
Phase 01 evidence and does not retroactively change it.

## P1 — Establish lane-state semantics

Author original 64-lane Wave64/EXEC expected outputs first
([#20](https://github.com/ranveersoni98/Xenora/issues/20)).
Cover full, empty, alternating and asymmetric masks, lanes 0/31/32/63,
nested save/restore, and single scalar side effects. Use AMD specifications
and an independent CPU oracle. Then choose one bounded EXEC/VCC
operation on Wave32 with exact inactive-lane preservation and controlled
rejection. Expand to divergence and Wave64 only after explicit mask
semantics, lane mapping and independent readback are established
([#24](https://github.com/ranveersoni98/Xenora/issues/24)).

Next, add checked descriptor, buffer and image access with lifetime
tests. Extend stages and render targets through exact pixel checks.
X4's host-authored render pipeline establishes a guest-driven draw;
guest RDNA vertex/fragment translation is a separate milestone.
For each change use public spec, original implementation, independent
oracle, LLVM where applicable, real Metal execution and independent review.

## P2 — Expand guest execution

Use [#38](https://github.com/ranveersoni98/Xenora/issues/38) for one
measurable register/flags/fault instruction slice and
[#48](https://github.com/ranveersoni98/Xenora/issues/48) for a checked
mapping/resource-lifetime slice. Preserve the interpreter as a reference,
the bounded loader and explicit service ABI. Expand only as required by
an original test program. Check rejected ranges and store atomicity,
permissions, resource ownership through completion, and repeat results.
Keep production graphics independent of guest CPU code.

## P3 — Native ARM64 recompilation

Profile representative interpreter workloads first. Translate one
independently modeled basic block, compare state and fault outcomes
with the interpreter, and verify native code execution. Broader
translation needs executable memory and code publication policy,
invalidation, handling of guest ordering, exceptions and SIMD, and
matched correctness-gated performance results.

## P4 — Release assurance

- [G10](https://github.com/ranveersoni98/Xenora/issues/11) complete the
  full correctness-gated corpus and controlled idle AC protocol, with
  stage timing, cold/warm data, thermal/capability metadata and variability.
- [G11](https://github.com/ranveersoni98/Xenora/issues/12) apply the
  [attribution and binary audit](../licenses.md#runtime-redistribution-review)
  to the exact archive and source offer, and resolve Apple-derived
  redistribution rights.
- [G12](https://github.com/ranveersoni98/Xenora/issues/13) obtain authentic
  contributor attestations for the six recorded historical commits.
- [G15](https://github.com/ranveersoni98/Xenora/issues/16) final independent
  R1 and R2 reports on the exact candidate with all prerequisites met.

These gates remain release requirements. Unrelated isolated experiments
can proceed with accurate evidence and provenance.

## P5 — Genuine Xbox execution

Remaining work includes Xbox-specific executable and process contracts,
CPU execution and translated state, memory permissions, thread and
system services, command and descriptor decoding, GPU resource lifetime,
coherence and synchronization, shader stages, audio/input, and
compatibility and performance validation. Use public specifications
and suitable project test inputs. The
[user-file policy](../legal.md#user-provided-game-files) applies to
import. Track the technical compatibility path under
[#55](https://github.com/ranveersoni98/Xenora/issues/55). A genuine measured
GTA VI intro/title screen remains a long-term stretch goal.
