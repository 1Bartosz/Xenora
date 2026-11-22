# Independent review of the first RDNA2 fixture

AMD's RDNA2 ISA reference §8.1 defines indexed buffer addressing and the four-SGPR resource operand. Under Xenora's synthetic descriptors, the first instruction reads input word `i` into v1 and the store writes word `i` of the independent output. The vector memory wait permits the read's result to be consumed before the add. Section 12 gives the VOP1 move and carry-free unsigned addition semantics, so v2 receives `0x12345678` and v1 receives the sum reduced modulo `2^32`. The SOPK store wait completes the outstanding store. The SOPP end instruction also implicitly waits on RDNA2. Requiring both explicit waits is a conservative policy of this fixture compiler. These semantics are class B for RDNA2 and do not establish Xbox-specific applicability. [AMD RDNA2 ISA Reference Guide](https://docs.amd.com/v/u/en-US/rdna2-shader-instruction-set-architecture)

LLVM 23.1.6 and the production decoder agreed on all seven fixture instructions, including their bytes, lengths, offsets and printed operands. AMD defines hardware semantics. LLVM is class E evidence for the tool's encoding and decoding behavior.

```sh
LLVM_BIN="$(brew --prefix llvm)/bin" cargo test -p xenora-decoder --test gfx1030 -- --ignored --nocapture
CARGO_TARGET_DIR=target/semantic-review cargo test --manifest-path tests/validation/Cargo.toml
```

The reviewer computed the four known answers independently using widened integer addition, yielding `[305419896, 305419897, 305419895, 0]`. The same derivation matched the interpreter across 262,144 boundary-neighborhood inputs and 4,096 deterministic pseudorandom inputs. Compiler and interpreter acceptance agreed on all 352 single-bit perturbations of this fixture. Acceptance matching cannot establish equivalent execution for mutated programs.

The compiler and interpreter use the same restricted fixture ABI. The reviewed kernel emits an unsigned word addition and accesses only the selected input and output word. General instruction forms and full Wave32 execution semantics remain outside this review. GPU execution was not run by this reviewer. The tests do not satisfy the full independent-assurance or source mutation gates, and the reference decoder still needs an automated form-wide comparison against LLVM and the production decoder.

## Branch and floating-point fixtures

AMD RDNA2 ISA §12.4 defines equality as an SCC update, and §12.5 uses a signed dword displacement relative to the next instruction for the conditional and unconditional branches. In `bitwise-branch`, PC 20 with displacement 2 targets byte 32 because 7 differs from 6. The independent test also changes the comparator to 7 and checks the fallthrough's zero output and the selected MSL path. A displacement of 3 lands in a literal at byte 36 and both engines reject it. The compiler resolves this defined scalar input at translation time. Coverage applies to uniform forward control flow selected from known scalar values. [AMD RDNA2 ISA Reference Guide](https://docs.amd.com/v/u/en-US/rdna2-shader-instruction-set-architecture)

The mask in `float-exact` erases every bit except 22 before XOR with the representation of 1. Input bit 22 then selects exactly 1 or 3/2. Adding 5/2 and multiplying by 5/2 gives 35/4 or 10, with positive normal binary32 operands and intermediate and final values. The integer input words may have arbitrary exponent bits, including NaN and infinity patterns, because the mask erases them before reinterpretation. For the verifier's general restricted domain, products of two binary32 significands fit within 48 significant bits and the normal binary32 range is inside the binary64 exponent range even after multiplying. For positive sums, the rounded binary64 sum and its larger operand are within a factor of two, so subtraction is exact by Sterbenz's lemma. Recovering the smaller operand proves the binary64 addition exact before the binary32 representability check. Tests reject a subnormal operand, a midpoint requiring binary32 rounding, binary64 absorption of the smallest normal binary32 operand, and negative infinity.

The locked validation package passed all four tests. The reviewer used CPU execution only. No independent GPU conclusion follows from these tests.

## Independent Phase 01 assurance

R2 reviewed the supported gfx1030 subset at `aff46b0` and signed off on
303 three-way comparisons over 15 instruction forms, three malformed
encoding probes and six supported-architecture encodings outside Xenora's
subset. Section 13 of AMD's RDNA2 guide describes the relevant encoding
layouts. LLVM provides class E tool evidence and the AMD architecture
guide provides class B semantics for the documented AMD target.
[AMD RDNA 2 ISA reference](https://docs.amd.com/v/u/en-US/rdna2-shader-instruction-set-architecture)

| Instructions | Independent evidence |
| --- | --- |
| `buffer_load_dword`, `buffer_store_dword` | The synthetic descriptors select separate input and output lane words. High VGPR renaming preserves outputs. Skipped invalid bindings, overlapping input operations and a second store are rejected. |
| `s_waitcnt vmcnt(0)`, `s_waitcnt_vscnt null, 0`, `s_endpgm` | Pending loads and stores block premature reads and termination. An early end is rejected even when skipped by a branch. A completed store does not discharge a later load. |
| `v_mov_b32_e32`, `v_add_nc_u32_e64` | Register moves preserve words, and a widened sum reduced modulo 2^32 supplies independent expected outputs. Coverage includes high VGPRs, in-place addition and wrap to zero. |
| `s_mov_b32`, `s_cmp_eq_u32`, `s_cbranch_scc0`, `s_branch` | SGPR and literal values select independently known branch outputs for both SCC results. The descriptor/ordinary SGPR boundary and invalid branch targets have negative cases. |
| `v_and_b32_e32`, `v_xor_b32_e32` | Fixed masks are checked with all-ones and mixed-bit words. The float fixture's mask keeps only bit 22, then XOR supplies the exponent of 1.0. |
| `v_add_f32_e32`, `v_mul_f32_e32` | Independent integer-significand verification covers exact products and sums, representational midpoint, underflow, overflow, absorption, and excluded operand classes. |

The second interpreter extracts the 24-bit significand and exponent.
For positive sums, an exponent gap of at least 24 makes the smaller
operand strictly smaller than one ULP of the larger. Smaller gaps allow
an exact integer sum in 48 bits. A product of two significands also fits
in 48 bits. A result is representable in the supported normal domain
when its reduced significant width is at most 24 bits and its normalized
exponent is between -126 and 127. This independent computation avoids
the previous secondary oracle's binary64 absorption of `1.0 + 2^-126`.

R1 independently rederived 20 expected boundary cases with rational
arithmetic and found no critical arithmetic issue in the reviewed draft.
The original draft passed 10 isolated Rust tests against the candidate
crates. The integrated suite adds witnesses for seven independent-oracle
survivors and passes 13 locked tests. The high-register and alias cases
compare both interpreters' readbacks and compiler acceptance. Native GPU
evidence continues to cover the three original fixtures. See the
[mutation report](../../docs/testing/mutation.md) for sensitivity results.
