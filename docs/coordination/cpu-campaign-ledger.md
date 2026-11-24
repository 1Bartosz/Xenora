# Xenora CPU/Runtime Expansion Campaign — Progress Ledger

Starting commit: `3ff9ddd` (321 tests, baseline).
Current commit (branch `feat/cpu-runtime-expansion`): `bfd14e4`.
Main at `7e09c9c`.

## New substantive commits (5)

1. `a953ba9` — fix(cpu): enforce bounded prefix and architectural instruction length in probe, interpreter and DBT (#69)
2. `bc629f5` — merge origin/main (typed Prefixes struct, completes #62)
3. `9cd6f43` — fix(cpu): bound interpreter segment prefix run by the architectural instruction length
4. `7e09c9c` — feat(cpu): implement MOVSX/MOVSXD (0x63) semantics in the interpreter (#117)
5. `bfd14e4` — feat(dbt): implement native ARM64 MOVSXD register lowering

## Verification ledger (all exit codes real)

- `cargo fmt --all -- --check` → 0
- `cargo clippy --locked --workspace --all-targets --all-features -- -D warnings` → 0
- `cargo test --locked --workspace` → 0; 381 passed, 0 failed

## Completed batches

- Phase A #1: Native ARM64 MOVSXD register lowering (DBT, `SXTW`, llvm-mc-verified encoding)
- #69: Bounded x86 prefix decoding across probe/interpreter/DBT (fuzzer crash minimized and fixed)
- #62: Typed Prefixes architecture integrated
- #117: MOVSX/MOVSXD interpreter semantics (interpreter + DBT)

## Remaining implementation batches (order follows the campaign plan)

Phase A: MOVSXD memory operands (#2); REX.X SIB index semantics (#63 — claimed by Swastickc);
MOVSX destination widths (#65), MOVZX/MOVSX interpreter-DBT parity (#66); RIP-relative addressing
verification (#67); high-byte registers (#68); architectural flags across blocks (#70); parity
flag (#71); FS/GS segment overrides (#72); LEA (#73).

Phase B (codegen): metadata registry + generator (validate/generate/--check modes),
generated dispatch for representative instructions, encoding/ambiguity verification,
deterministic coverage reporting.

Phase C (expansion): 16-bit operand forms (#118), CMP/TEST memory (#119), memory ALU (#120),
ADC/SBB (#195), shifts/rotates (#121), MUL/DIV/IDIV (#122), CMOVcc/SETcc (#123),
bit scans/POPCNT, XMM state (#124), SSE2 NEON lowering (#30 in plan).

Phase D (runtime): import-gen metadata, ABI adapters, CRT memory/string/math ops,
heap/TLS/clock/filesystem/sync services (#31–#44), workloads and coverage (#45).

## Independent reviews

Not completed (per policy, reported honestly): no independent reviewer runs were available in
this session; batches were reviewed by the implementer and verified by focused + workspace
regression tests.

## Known limitations

- `cpu_decode` fuzz: smoke run after the prefix fix; the 150 s run's crash
  (`crash-9c788e52…`, minimized to `[0x65×12, 0x73, 0x65×4]`) was reproduced at unit level and
  fixed by the general >15-byte enforcement in `decode_one`. Full 600 s historical campaigns not re-run.
- DBT support for opcodes other than the current set is still narrow (no memory-operand
  MOVSXD in DBT yet).
- No performance benchmarks recorded in this ledger (CPU correctness batches).

## Next implementation target

Phase A #2: MOVSXD memory operands in the interpreter (`crates/loader/src/cpu.rs`), then
Phase A #3 (REX.X SIB index semantics, unclaimed), then Phase B codegen infrastructure.
