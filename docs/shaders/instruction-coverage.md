# gfx1030 instruction coverage

For counts of every gfx1030 form in AMD's metadata against forms Xenora can execute, run `xenora shaders coverage`. Its "discovered" column is metadata coverage, not execution support.

The decoder accepts an explicitly tagged, little-endian gfx1030 Wave32
stream with 32 lanes per wave under the [synthetic memory ABI](../adr/0003-demo-memory-model.md).
Recognition means that the opcode mask matches. Decode additionally checks
operands, flags, instruction width and any literal word. Register
initialization and memory safety require verification after decoding.
CPU and GPU entries refer to the original synthetic fixture workloads.
The wider LLVM operand matrix exercises decoding only. ABI means application binary interface. VGPRs are vector general-purpose registers and SGPRs are scalar general-purpose registers.

<!-- status:instructions:start -->
| Form | Operands | Size or word | Recognized | Decoded | LLVM | Lowering | Reference | CPU | GPU |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| `buffer_load_dword` | MUBUF, VGPR data/index, four normal SGPRs, `idxen`, zero offsets and remaining flags | 8 B | yes | yes | three-way passed | MSL load | implemented | fixture verified | fixture verified |
| `s_waitcnt vmcnt(0)` | SOPP, exact immediate `0x3f70`; other counters disabled by their maximum values | 4 B | yes | yes | three-way passed | verified load wait | implemented | fixture verified | fixture verified |
| `v_mov_b32_e32` | VOP1, VGPR destination, VGPR or inline integer or literal source, plus the inline bit pattern for +1.0f | 4 or 8 B | yes | yes | three-way passed | constant or register value | implemented | fixture verified | fixture verified |
| `v_add_nc_u32_e64` | VOP3, VGPR destination and two VGPR inputs, modifier fields clear, third source field encoded as inline zero | 8 B | yes | yes | three-way passed | MSL unsigned add | implemented | fixture verified | fixture verified |
| `buffer_store_dword` | MUBUF, VGPR data/index, four normal SGPRs, `idxen`, zero offsets and remaining flags | 8 B | yes | yes | three-way passed | MSL store | implemented | fixture verified | fixture verified |
| `s_waitcnt_vscnt null, 0` | SOPK, exact null source and zero immediate | 4 B | yes | yes | three-way passed | verified store wait | implemented | fixture verified | fixture verified |
| `s_endpgm` | SOPP, zero immediate | 4 B | yes | yes | three-way passed | verified termination | implemented | fixture verified | fixture verified |
| `s_mov_b32` | Normal SGPR or EXEC/VCC half destination and normal SGPR, inline integer or literal source | `BE880387` | yes | yes | three-way passed | resolved scalar state | implemented | fixture verified | fixture verified |
| `s_cmp_eq_u32` | Normal SGPR or integer operands, with at most one source selecting a literal | `BF068708` | yes | yes | three-way passed | resolved SCC | implemented | fixture verified | fixture verified |
| `s_cbranch_scc0` | Nonnegative signed dword displacement | `BF840002` | yes | yes | three-way passed | resolved conditional branch | implemented | fixture verified | fixture verified |
| `s_branch` | Nonnegative signed dword displacement | `BF820001` | yes | yes | three-way passed | resolved branch | implemented | fixture verified | fixture verified |
| `v_and_b32_e32` | Two VGPR sources and one VGPR destination | `36020702` | yes | yes | three-way passed | MSL bitwise AND | implemented | fixture verified | fixture verified |
| `v_xor_b32_e32` | Two VGPR sources and one VGPR destination | `3A020702` | yes | yes | three-way passed | MSL bitwise XOR | implemented | fixture verified | fixture verified |
| `v_add_f32_e32` | Two VGPR sources and one VGPR destination | `06020702` | yes | yes | three-way passed | MSL FP32 add | implemented | fixture verified | fixture verified |
| `v_mul_f32_e32` | Two VGPR sources and one VGPR destination | `10020702` | yes | yes | three-way passed | MSL FP32 multiply | implemented | fixture verified | fixture verified |
| `v_or_b32_e32` | Two VGPR sources and one VGPR destination | `3803FF01` | yes | yes | three-way passed | MSL bitwise OR | implemented | fixture verified | fixture verified on Apple M5 |
| `v_lshlrev_b32_e32` | Two VGPR sources and one VGPR destination | `34020501` | yes | yes | three-way passed | MSL logical left shift with a five-bit count | implemented | fixture verified | fixture verified on Apple M5 |
| `v_lshrrev_b32_e32` | Two VGPR sources and one VGPR destination | `2DFFFF01` | yes | yes | three-way passed | MSL logical right shift with a five-bit count | implemented | fixture verified | fixture verified on Apple M5 |
| `v_ashrrev_i32_e32` | Two VGPR sources and one VGPR destination | `31FFFF01` | yes | yes | three-way passed | MSL signed right shift with a five-bit count | implemented | local known-answer test passed | local Apple M5 readback passed; acceptance pending |
| `s_cbranch_execz` | Nonnegative signed dword displacement | `BF880001` | yes | yes | three-way passed | resolved Wave32 EXEC_LO zero check | implemented | local known-answer test passed | local Apple M5 readback passed; acceptance pending |
| `s_cbranch_vccz` | Nonnegative signed dword displacement | `BF860001` | yes | yes | three-way passed | resolved Wave32 VCC_LO zero check | implemented | local known-answer test passed | local Apple M5 readback passed; acceptance pending |
<!-- status:instructions:end -->

## Automated three-way comparison

LLVM 23.1.6, the production decoder and the independent reference decoder
matched 338 instructions across eight streams: the six synthetic fixtures,
a high-register sequence and a 272-instruction matrix assembled by LLVM.
The harness checks byte offsets, widths, encoded bytes from LLVM, mnemonic
and operands, and the typed reference operation against the production
operation. It covers all 21 forms in this document.

The matrix checks the 81 inline integer selectors in VOP1, SOP1 and SOPC,
forced literals that could otherwise be inline, literal selection in either
SOPC operand, high SGPR and VGPR indices, MUBUF descriptor and index
operands, and zero and nonzero forward branch displacements. The decoder
trace exposes MUBUF operands independently of the synthetic execution ABI,
which still requires the original index and resource bindings.

Five malformed LLVM probes produced diagnostics, and 19 valid encodings
or disassemblable branch sequences were deliberately rejected by both Xenora
decoders. Both Xenora decoders also reject all 37 interior truncation offsets
in the add fixture and every case in the unsupported-encoding list.
These case counts overlap where the same bytes are used in distinct tests.
The truncated VOP1 literal is checked in Xenora only. LLVM 23.1.6's direct
literal-truncation probe crashed, so it is outside the automated LLVM
negative matrix.

This is form and operand-class coverage. The matrix does not enumerate
every register combination, literal value or forward displacement. LLVM
supplies encoding and disassembly evidence. The reference and semantic
review tests exercise execution, including both SCC outcomes and the exact
FP32 domain. The native GPU demo verifies the three executable fixtures.

```sh
scripts/assemble-fixture.sh --check
LLVM_BIN="$(brew --prefix llvm)/bin" cargo test --locked -p xenora-decoder --test gfx1030 -- --include-ignored --nocapture
cargo test --locked -p xenora-ref -p xenora-semantic-review
scripts/demo.sh
```

The [source notes](../research/rdna2.md) record the XML revision and license.

## Operand and execution notes

AMD RDNA 2 ISA §§12.3–12.5 and 12.7 define the scalar and vector forms. AMD's pinned XML
and native LLVM 23.1.6 agree on the representative words. The target of each
branch is the byte offset after its four-byte instruction plus four times the
sign-extended 16-bit displacement. This subset requires forward targets at
instruction boundaries. The vector forms carry two VGPR sources and no
modifiers. Floating-point arithmetic and its mode constraints are execution
requirements outside the decoder.
