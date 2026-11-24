# Xenora CPU/Runtime Expansion Campaign — Progress Ledger

Evidence below is local to the named host unless stated. Nothing here is independently reviewed yet.

## Continuation checkpoint

- Continuation branch base: `0836936` (tip of `feat/cpu-runtime-expansion`), merged with main `d22f046` (#272, #273, #274) as `56febbb`. The merge had no conflicts.
- All recorded checkpoints (`a953ba9`, `9cd6f43`, `7e09c9c`, `bfd14e4`, `0836936`) exist upstream. The earlier session's uncommitted MOVSX patch was not available and was not used.
- Host: Apple M5 Pro, macOS, Rust 1.100.0, Apple clang 17.0.0. No Homebrew LLVM, `llvm-mc` or full Xcode.
- Baseline of the merge: `cargo test --locked --workspace --no-fail-fast` exited 101 with 392 passed, 1 failed, 21 ignored. The failure (`probe_fuzz_harness_minimized_artifact`) also occurs on `0836936`: the test read an untracked file.

## Corrections to earlier entries

- The earlier claim of 381 passed and 0 failed did not hold for a fresh checkout (see above).
- The SXTW encoding recorded as verified was wrong for most source registers.
- #65, #66 and #117 were marked complete prematurely. They stay open until reviewed against their own acceptance criteria.

## Milestones

| # | Commit subject | Scope | Evidence |
| --- | --- | --- | --- |
| 1 | cpu: correct the sign- and zero-extension family | MOVZX/MOVSX/MOVSXD, MOV-immediate regressions, SXTW, 32-bit access width, RIP-relative and LEA addressing, self-contained prefix test | `tests/movsxd.rs`: 9 tests over 93 LLVM-assembled register forms and 32 MOV immediates, each run by the interpreter and as native translated code; 16 of 16 targeted mutations caught |

Encodings: `clang -target x86_64-apple-macos -c` and `clang -target arm64-apple-macos -c`, read back with `objdump -d`.

## Known defects, not yet fixed

- The DBT keeps guest flags only in host NZCV: they do not survive helper calls or block boundaries, and register ADD/SUB/XOR set none (#70).
- JB/JAE/JBE/JA use ARM carry polarity, which is inverted for x86 subtraction (#64).
- The probe marks several forms as interpreter-supported that the interpreter rejects (NOP, most Jcc, FF group).
- Combined FS and GS prefixes: precedence is unverified (#72).

## Pending acceptance

- Two independent reviews required by `AGENTS.md`: not performed.
- Issues touched here have not been claimed on GitHub.
- LLVM AMDGPU fixture gates, native GPU gates, app bundle and fuzz campaigns: not run on this host.

## Next task

Store guest flags architecturally in the DBT and correct unsigned conditions (#64, #70), then the shared decoder and instruction registry.
