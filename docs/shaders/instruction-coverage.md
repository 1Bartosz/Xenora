# gfx1030 instruction coverage

The decoder accepts an explicitly tagged, little-endian gfx1030 Wave32
stream under the [synthetic memory ABI](../adr/0003-demo-memory-model.md).
Recognition means that the opcode mask matches. Decode additionally checks
operands, flags, instruction width and any literal word. Register
initialization and memory safety require verification after decoding.

| Form | Accepted encoding and operands | Size | Recognized | Decoded | LLVM fixture comparison |
| --- | --- | --- | --- | --- | --- |
| `buffer_load_dword` | MUBUF, VGPR data/index, four normal SGPRs, `idxen`, zero offsets and remaining flags | 8 B | yes | yes | pass |
| `s_waitcnt vmcnt(0)` | SOPP, exact immediate `0x3f70`; other counters disabled by their maximum values | 4 B | yes | yes | pass |
| `v_mov_b32_e32` | VOP1, VGPR destination, VGPR or inline integer or literal source, plus the inline bit pattern for +1.0f | 4 or 8 B | yes | yes | pass for literal, inline zero and inline +1.0f |
| `v_add_nc_u32_e64` | VOP3, VGPR destination and two VGPR inputs, modifier fields clear, third source field encoded as inline zero | 8 B | yes | yes | pass |
| `buffer_store_dword` | MUBUF, VGPR data/index, four normal SGPRs, `idxen`, zero offsets and remaining flags | 8 B | yes | yes | pass |
| `s_waitcnt_vscnt null, 0` | SOPK, exact null source and zero immediate | 4 B | yes | yes | pass |
| `s_endpgm` | SOPP, zero immediate | 4 B | yes | yes | pass |

The production decoder matched all three fixture streams with LLVM 23.1.6.
The comparison checks instruction order, byte offsets, lengths, re-encoded
bytes, mnemonics and operands. It normalizes LLVM's spelling of zero in
`s_waitcnt_vscnt`. This comparison covers the production decoder and LLVM.
The independent decoder and execution stages require their own evidence.

```sh
LLVM_BIN="$(brew --prefix llvm)/bin" scripts/assemble-fixture.sh --check
LLVM_BIN="$(brew --prefix llvm)/bin" cargo test -p xenora-decoder --test gfx1030 -- --ignored --nocapture
```

The [source notes](../research/rdna2.md) record the XML revision and license.

## Expansion variants

AMD RDNA 2 ISA §§12.3–12.5 and 12.7 define these forms. AMD's pinned XML
and native LLVM 23.1.6 agree on the representative words. The target of each
branch is the byte offset after its four-byte instruction plus four times the
sign-extended 16-bit displacement. This subset requires forward targets at
instruction boundaries. The vector forms carry two VGPR sources and no
modifiers. Floating-point arithmetic and its mode constraints are execution
requirements outside the decoder.

| Form | Operands | Representative LLVM encoding | Recognized | Decoded | LLVM fixture comparison |
| --- | --- | --- | --- | --- | --- |
| `s_mov_b32` | Normal SGPR destination and normal SGPR, inline integer or literal source | `BE880387` | yes | yes | pass |
| `s_cmp_eq_u32` | Normal SGPR or integer operands, with at most one source selecting a literal | `BF068708` | yes | yes | pass |
| `s_cbranch_scc0` | Nonnegative signed dword displacement | `BF840002` | yes | yes | pass |
| `s_branch` | Nonnegative signed dword displacement | `BF820001` | yes | yes | pass |
| `v_and_b32_e32` | Two VGPR sources and one VGPR destination | `36020702` | yes | yes | pass |
| `v_xor_b32_e32` | Two VGPR sources and one VGPR destination | `3A020702` | yes | yes | pass |
| `v_add_f32_e32` | Two VGPR sources and one VGPR destination | `06020702` | yes | yes | pass |
| `v_mul_f32_e32` | Two VGPR sources and one VGPR destination | `10020702` | yes | yes | pass |
