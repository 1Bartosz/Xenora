# SOP2 scalar ALU subset

Xenora decodes and executes seven gfx1030 SOP2 forms. Each takes 32-bit
operands, writes a normal SGPR (s0-s105) and sets SCC.

| Form | D | SCC |
| --- | --- | --- |
| `s_add_u32` | S0 + S1 | carry-out of bit 31 |
| `s_sub_u32` | S0 - S1 | borrow (S1 > S0) |
| `s_addc_u32` | S0 + S1 + SCC | carry-out of bit 31 |
| `s_subb_u32` | S0 - S1 - SCC | borrow (S1 + SCC > S0) |
| `s_and_b32` | S0 & S1 | D != 0 |
| `s_or_b32` | S0 \| S1 | D != 0 |
| `s_xor_b32` | S0 ^ S1 | D != 0 |

Sources are normal SGPRs, inline integers (-16 to 64) or one literal word.
`v_mov_b32_e32` may read a normal SGPR, so the results can reach vector code.

## Sources

- Opcodes, operand fields and which forms read and write SCC come from the
  pinned AMD RDNA 2 XML (`crates/isa/generate_rdna2.py`). Its descriptions say
  that additions "store the carry-out bit into SCC", ADDC adds "a carry-in bit
  from SCC", SUBB subtracts "the carry-in bit", and the bitwise forms "set SCC
  iff the result is nonzero".
- For subtraction, the XML's "carry-out" does not say whether SCC is the borrow
  or its complement. LLVM's AMDGPU backend settles it. It lowers unsigned
  subtract-with-overflow (`S_USUBO_PSEUDO`) to `s_sub_u32` and reads SCC as the
  overflow. It also lowers 64-bit subtraction to `s_sub_u32` followed by
  `s_subb_u32` (`SIISelLowering.cpp`), which is only correct if SCC is the
  borrow and SUBB subtracts it. LLVM 23.1.6 also checks every encoding here
  against the decoder and the reference (`compares_fixture_instructions_against_llvm`).

## Not supported

- `s_add_i32` and `s_sub_i32`. The XML also says they store "the carry-out
  bit", which for signed operands could mean the carry or signed overflow.
  The RDNA 2 ISA guide's SOP2 pseudocode is understood to define signed
  overflow. That has not been checked against the guide here, so both stay
  rejected until a follow-up confirms it and adds overflow vectors.
- Every other SOP2 form, including the 64-bit bitwise forms.
- Destinations other than s0-s105 (VCC, EXEC, TTMP and M0).
- Sources other than normal SGPRs, inline integers and one literal.
- Both sources selecting a literal.
- ADDC or SUBB before SCC is set.
- Writes to the fixture's resource descriptors s0-s7.

## How it runs

Scalar state in a compute fixture is wave-uniform and known while the code is
translated. As with `s_mov_b32` and `s_cmp_eq_u32`, the translator evaluates
SOP2 forms itself. It resolves SCC-dependent branches and emits SGPR values
read by `v_mov_b32_e32` as constants (`fixtures/golden/scalar-alu.ir`). So the
IR and MSL contain the results, not scalar instructions.

## Verification

`fixtures/rdna2/scalar-alu.s` is an original program, assembled by LLVM. It
chains a carry through `s_add_u32`/`s_addc_u32` and a borrow through
`s_sub_u32`/`s_subb_u32`. It also covers aliased destinations, inline and
literal operands, bitwise results that set and clear SCC, and an
`s_cbranch_scc0` that depends on them. Its expected output, input ^ 0x8000000e,
was derived by hand.

`tests/validation/known_answers.rs` runs every supported form over 10 boundary
operands squared, with SCC clear and set. That is 1,400 programs. The same file
covers aliasing, inline and literal operands, and rejected forms. Expected
values come from exact integer definitions in the test. Three implementations
must agree with them:
- the translator (resolved branch and emitted constant);
- the reference interpreter (exact integers, reduced modulo 2^32);
- the independent interpreter (a bit-serial ripple-carry adder).

`scalar_alu_native_metal_known_answers` runs the fixture, the 1,400 programs
and the aliasing and immediate cases on native Metal with API and shader
validation. `scripts/test-gpu.sh` runs it. This is local evidence from one
Apple M5 Pro; independent acceptance is pending.
