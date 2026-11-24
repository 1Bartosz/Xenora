# x86-64 instruction registry

The static PE probe, the interpreter and the native ARM64 dynamic binary translator (DBT) decode guest code through one function, `x86_decode::decode`. Its tables are generated from `crates/loader/x86/instructions.toml`.

| Part | File | Role |
| --- | --- | --- |
| Registry | `crates/loader/x86/instructions.toml` | Opcode map, opcode, ModRM digit, operand encoding, size, immediate, flag effects and public reference. Data only. |
| Generator | `crates/loader/generate_x86.py` | `validate`, `generate` and `--check`. Rejects overlapping encodings, opcodes that are prefixes, contradictory operand rules, unknown vocabulary and flags both written and undefined. |
| Tables | `crates/loader/src/x86_table.rs` | Generated static tables. No metadata is parsed at run time. |
| Semantics | `crates/loader/src/x86_exec.rs`, `x86_simd.rs`, `x86_float.rs`, `softfloat.rs` | Rust primitives for integer, vector and floating-point forms. Every instruction is all-or-nothing. |
| Lowering | `crates/loader/src/dbt.rs` | ARM64 for the forms `is_native` accepts. Every other form calls the shared semantics and is counted in `DbtMetrics::interpreter_fallbacks`. |

## Adding a form

1. Add the form, and its operation if new, to the registry with an Intel SDM or AMD APM reference.
2. Run `python3 crates/loader/generate_x86.py generate`.
3. Implement the operation in `x86_exec.rs`. Leave flags the manuals call undefined unchanged.
4. Add assembly text to `crates/loader/tests/x86/assemble.py`, run it with `generate`, and extend the reference in `tests/x86_integer.rs` or `tests/x86_vector.rs`. A declared form without a fixture fails `every_declared_form_is_executed_by_a_fixture`.
5. Lower it natively only if the ARM64 result and flags match. The same tests require native forms never to call the helper, and helper forms always to.

A declared form is decodable, not implemented. Count a form as supported only when the tests above cover it.

## Independence

The references in `tests/x86_integer.rs` and `tests/x86_vector.rs` read only LLVM assembly text. They do not use the encodings, registry, decoder or production semantics, and work bit by bit or on byte arrays. They were written by the same contributor as the implementation, so independent review is still required.

## Guest flags

The DBT stores CF, ZF, SF, OF, PF and AF in `CpuState::flags`. It skips the store only when a later instruction in the block overwrites all six before anything can read them, fault or leave the block.

## Vector state

XMM registers live in `CpuState::xmm`, lane 0 in the low bits, and MXCSR in `CpuState::mxcsr`. A mandatory 0x66, F3 or F2 prefix selects a form; the generator rejects an opcode where 0x66 could be both a mandatory prefix and an operand size. A 16-byte memory operand must be aligned unless its form says otherwise, and a violation exits with `GeneralProtection` before any memory access. Non-temporal stores are ordinary stores.

## Floating point

`softfloat.rs` follows the Intel SDM (Vol. 1, 4.8, 4.9 and 11.5) without host floating point. Expected values come from exact fractions in `tests/x86/float_reference.py`. Some behaviour is implemented one way but not asserted, because the sections used leave it open or it was not found there:

- a denormal dividend over zero raises divide-by-zero only, not also denormal-operand;
- flush-to-zero also flushes a tiny result that is exact, raising underflow and precision;
- DAZ applies before conversion to an integer;
- an unmasked overflow also records precision.

None of it has been compared with x86 hardware.

## Rejected on purpose

LOCK, F2 or F3 where it is neither PAUSE nor part of a declared form, F2 combined with F3, address-size overrides, FS combined with GS, an ignored segment override after FS or GS, 16-bit stack and branch forms, 16-bit BSWAP and MOVSXD, bit tests of memory by a register index, RCL, RCR, SHLD, SHRD, string instructions, MMX, the RCP and RSQRT approximations, SSE3 and later, VEX, atomics and fences. The DBT also refuses raw SYSCALL.
