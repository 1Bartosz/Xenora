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
| 2 | cpu: share one generated decoder and store guest flags | Registry, generator, shared decoder and semantics, integer expansion, architectural DBT flags, truthful probe support | `tests/x86_integer.rs`, `tests/arm64_encodings.rs`, `tests/x86_generator`; 37 targeted mutations: 36 caught (4 only after new tests), and one equivalent, whose redundant check was removed |

Encodings: `clang -target x86_64-apple-macos -c` and `clang -target arm64-apple-macos -c`, read back with `objdump -d`.

## Verification of milestone 2 (local, real exit codes)

| Gate | Result |
| --- | --- |
| `cargo fmt --all -- --check`; strict Clippy, workspace, all targets and features | exit 0; exit 0 |
| `cargo test --locked --workspace --no-fail-fast` | exit 0: 409 passed, 0 failed, 21 ignored |
| `generate_x86.py --check`, `assemble.py --check`, generator tests (8) | exit 0 |
| RDNA2 metadata, semantic registry, coverage and status checks; their Python tests | exit 0 |
| `scripts/audit/{claims,dependencies,native}.sh`, release build, `shaders coverage` | exit 0 |
| `scripts/test-gpu.sh` | first run exit 124 (its own timeout, in `sop2_arith`, straight after a release build); rerun exit 0: 69 passed, 0 failed, 1 ignored |
| `scripts/demo.sh`, `scripts/build-app.sh`, `--smoke-frame`, `--inspect` | exit 0 |
| `scripts/assemble-fixture.sh --check`; ignored LLVM tests in `xenora-isa` and `xenora-decoder` | not verified: need Homebrew LLVM (`llvm-mc`) |
| Fuzz and mutation campaigns | not run |

Coverage: 56 operations, 184 declared forms, 279 opcode slots. 985 LLVM-assembled instructions execute every form except control flow and exits, which have their own tests.

`dbt_bench` on an Apple M5 Pro, 101 samples, three runs each of `5ab561b` and milestone 2, middle run: interpreter 12.69 to 11.12 MIPS, cold DBT 10.43 to 8.86 MIPS, warm DBT 468.83 to 424.86 MIPS, native code 242,608 to 290,184 bytes. The slowdown comes with the new work of storing six flags and the shared decode; the earlier figures belong to code that computed wrong flags. Not optimized yet. These are synthetic and say nothing about games.

## Known limitations

- Only the integer forms in `x86/instructions.toml` exist. XMM and MXCSR state, string instructions, RCL, RCR, SHLD, SHRD, LOCK, fences and atomics are unsupported.
- 8- and 16-bit ALU operations, shifts, MUL, DIV and bit operations run through the shared semantics in the DBT, not as ARM64.
- The reference and the implementation share an author.
- New CPU checks are not in CI; `.github` was left unchanged.

## Pending acceptance

- Two independent reviews required by `AGENTS.md`: not performed.
- Issues touched here have not been claimed on GitHub and stay open.

## Next task

XMM and MXCSR architectural state with SSE2 moves and bitwise operations (#124, #125), then native lowering of 8- and 16-bit ALU and shifts, then runtime resolution and typed imports. Nothing in those areas is started.
