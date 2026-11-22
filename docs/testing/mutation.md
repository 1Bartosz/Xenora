# Phase 01 mutation testing

G7 remains open. The source campaign covers the decoder, fixture parser,
verifier, lowering and reference interpreter through the default CPU workspace
tests. The independent semantic and oracle campaigns are separate requirements.

## Historical source campaign

The raw outcomes and mutation diffs are under the ignored
`artifacts/mutation/campaign-20261122/mutants.out/`. The campaign used the
source at `a34dadc` with workspace tests, two workers and a 300-second timeout.

| Crate | Generated | Caught | Missed | Unviable | Timeout | Conservative score before equivalence review |
|---|---:|---:|---:|---:|---:|---:|
| Decoder | 116 | 93 | 10 | 12 | 1 | 89.4% |
| Reference | 171 | 139 | 27 | 5 | 0 | 83.7% |
| Shader | 100 | 63 | 29 | 8 | 0 | 68.5% |
| Total | 387 | 295 | 66 | 25 | 1 | 81.5% |

The decoder timeout changed `offset += byte_len` to multiplication. The
offset starts at zero and remains zero after each successful decode. The
historical test phase timed out after 300 seconds following a successful build.
It stays in the denominator as a timeout.

## Reproduction

The targeted rerun matches the exact identities in `missed.txt` and
`timeout.txt`. The full command and outputs are in
`artifacts/mutation/phase01-survivor-rerun/`. Its 30-second timeout distinguishes
the known non-progress case without waiting another five minutes.

Run the full original scope with a fresh output directory after the targeted
rerun and review.

```sh
CARGO_BUILD_JOBS=2 scripts/mutation.sh artifacts/mutation/final-source
```

Count kills over all eligible non-equivalent mutants, including timeouts and
unexecuted mutants. Record invalid mutants and independently reviewed
equivalences separately. A source score cannot substitute for the native
semantic campaign or the validation-fixture oracle campaign.

## R2 review of the historical survivors

Locations refer to each crate's `src/lib.rs` at `a34dadc`. The checked
identities and original outcomes remain in `outcomes.json`. R2 identified
53 genuine test gaps, 13 equivalent survivors and the separate timeout.

| Crate and locations | Proof of equivalence |
|---|---|
| Decoder 352:17 | The descriptor base is a multiple of four. Equality with 102 is unreachable, so `> 102` and `>= 102` are identical. |
| Reference 192:40, 193:42, 197:42, 203:40, 204:47 | OR and XOR pack disjoint bit fields. At each site the shifted contribution occupies bits cleared in the unshifted field. |
| Shader 127:44, 128:42, 137:43 | A bit cannot be known both zero and one. The OR operands are disjoint in each case, including the two ways to establish an XOR output bit. |
| Shader 141:37 | Unknown bits complement the union of known-zero and known-one bits, so they share no bits with the known-one mask. |
| Shader 183:9 | For a singleton result the masks already fully specify its value. The fallback reconstructs the same two identical candidates that the deleted arm would store explicitly. |
| Shader 275:13, 279:13 | The removed arm and weakened predicate affect private defense-in-depth. The sole caller supplies decoder output, which independently rejects backward branches and targets outside instruction boundaries before calling the verifier. |

R2 identified diagnostic, scalar and vector selector, fixture limit, transfer
ordering, reserved register and exact FP32 verifier regressions for the
remaining survivors. The targeted rerun records which tests actually
distinguish each mutant.
