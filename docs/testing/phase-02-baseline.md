# Dual-track baseline

The resumed checkout started clean on main at
`4b9a4cecf35c8d4d90d8ecdb9c1510e7bd260102`, tracking origin/main.
The diff since `5c479005b0e0631a608a369b9e41327da61bb61e` contains three
documentation files. The baseline reuses the implementation evidence from
that commit and retains the original attribution for every run. The
[Phase 01 continuation](phase01-continuation.md) records G1–G15 individually.
R1 has also [signed off G8 for Phase 01](review-log.md#phase-01-evidence-recheck-at-4b9a4ce). G9 passed for the Phase 01 candidate. G10–G12 and G15 remain open. Phase 01 is not release-ready.

The resumed host reported arm64, 16 GiB RAM, 276 GiB available disk,
macOS 27.3 (26C5047e), Xcode 27.2 (27B52) and Rust 1.100.0.
The implementation evidence is from Apple M5. The archived command and
exit logs at `artifacts/assurance-5c47900/` report passing format, Clippy,
workspace, LLVM, fixtures, GPU, demo, semantic, native, claims, status and
dependency checks. The workspace ran 54 tests. The native suite ran 12
tests with validation enabled. Three original shader fixtures passed and
12 GPU semantic mutants were killed. Those are separate suites and their
counts do not estimate GPU dispatch totals. The oracle-only campaign
killed 44 of 44 mutants. The [integrated mutation and oracle review](mutation.md#integrated-source-campaign-at-4b9a4ce)
closes G5 and G7 for the Phase 01 candidate.

## Track A — GPU engineering

The [status manifest](../status.toml) enumerates 15 recognized and
decoded gfx1030 forms. All 15 contribute to the verified fixture path.
Forward scalar control is resolved when lifting to IR. The generated MSL
embodies that resolution without emitting a dynamic GPU branch. The
translated stage is compute, with lane-indexed 32-bit reads and writes to
two disjoint buffers. The original native offscreen render test provides
separate backend evidence. Preserve the [FP32 host evidence](../research/host-lab.md)
and the exact positive normal FP32 subset.

Candidate first slice, pending Phase 01 decoder/compiler assurance:
add the desktop gfx1030 VGPR-only integer OR form to the existing compute
path. An original fixture can take inputs
`[0x00000000, 0xffffffff, 0xaaaaaaaa, 0x80000000]`, OR each with
`0x55555555`, and expect
`[0x55555555, 0xffffffff, 0xffffffff, 0xd5555555]`.
Homebrew LLVM 23.1.6 `llvm-mc -triple=amdgcn-amd-amdhsa -mcpu=gfx1030
-show-encoding` assembled `v_or_b32_e32 v1, v2, v3` to bytes
`02 07 02 38` and the all-`v255` form to `ff ff ff 39`. This is
class E tool evidence. Before implementation, ISA and R2 must check the
AMD definition and independently derive encoding and operand expectations.
Use existing buffer bindings and negative operand tests. Acceptance
requires decoder/IR/MSL integration, independent CPU and GPU readback,
original regressions, changed-code assurance and matched measurements.
Keep the work in a reversible patch with the baseline fixtures available
unchanged. See the implementation checkpoint below for the resulting verification.

## Track B — guest execution

| Milestone | Baseline status |
| --- | --- |
| X0 Native host bootstrap | Native host and Metal capability evidence exists in Phase 01. A standalone guest runtime driver is proposed. |
| X1 First x86-64 instruction | Demonstrated on ARM64 macOS with both known-answer traces and six fault tests. |
| X2 Executable and synthetic service | Independently demonstrated on ARM64 with the original fixed-base PE32+ fixture and synthetic add-one service. |
| X3 Guest GPU command | Independently verified from the original PE through the shared loader, interpreter and native Metal. |
| X4 Guest-driven Metal frame | Independently verified on Apple M5 from original guest PE through RGBA8 texture readback. |
| X5 Authentic Xbox boundaries | Blocked on public or authorized interfaces and test inputs. |
| X6 First real-title validation | Blocked on X5 and suitable project test inputs. User import follows the [game file policy](../legal.md#user-provided-game-files). |
| X7 Genuine GTA VI intro/title screen | Blocked. The runtime path and suitable project test inputs are unavailable. |

Track B research can proceed while Phase 01 gates remain open. The
[CPU feasibility report](../research/phase02-cpu-feasibility.md) proposes
an original interpreter with a 16-instruction budget, two branch paths
and exact register, flag and memory expectations. Keep the original
CPU test independent of graphics crates until the CPU, loader and
synthetic service contracts are verified.

The [KyTx and Leonida review](../research/phase02-collaboration.md)
pins the inspected sources and proposes an original Wave64/EXEC
fixture as the first collaboration. It is a separate fixture proposal
from the initial Track A OR slice. Acceptance requires file-level
provenance and independent lane expectations. Leonida's decoder and
loader are design references. No upstream component was adopted.

The [ecosystem shortlist](../research/phase02-ecosystem.md) screens
eight additional upstream repositories and ranks five original
experiments. Its highest-ranked differential compute experiment can
use the selected OR slice with a bitwise mutation and independently
derived lane outputs. Emulator code, ports, tables and internal metadata are outside scope.

## Phase 02 acceptance

| Gate | Baseline disposition |
| --- | --- |
| P2-01 | Baseline captured. Phase 01 evidence and blockers are linked above. |
| P2-02 | KyTx, Leonida and eight additional projects screened. Original fixtures and public specifications are the implementation source. No emulator code, tables or metadata are in scope for adoption. |
| P2-03 | VGPR-only OR verified on Apple M5. |
| P2-04 | Workspace, LLVM and native GPU regression runs passed after OR integration. |
| P2-05 | Independent OR expected values matched CPU execution and Metal readback. |
| P2-06 | Integrated Phase 01 source campaign passed. Reassess changed Phase 02 inputs. |
| P2-07 | Matched performance comparisons pending a validated slice. |
| P2-08 | Native audit and Metal suite passed on the OR working tree. |
| P2-09 | OR status inventory updated. Remaining Phase 02 claims require review. |
| P2-10 | Independent Phase 02 R1 and R2 reports pending. |

## Implementation checkpoint

The working tree based on `4b9a4ce` adds VGPR-only
`v_or_b32_e32`. The verification snapshot and fixture hashes are in
`artifacts/phase02-or/`. The workspace ran 56 tests successfully.
The three opt-in tests were run separately. LLVM reported 312 three-way
comparisons across 16 forms and 15 malformed or out-of-subset probes.
The 12 native regression tests passed. With Metal API and shader
validation enabled, the original OR fixture matched all 131 expected
output words in one native dispatch. Clippy, fixture regeneration and
the 15-executable native audit passed. This verifies the bounded
feature on Apple M5. Changed-code fuzz and mutation evidence and
matched performance still remain open for Phase 02.

The [isolated X1 experiment](../../experiments/x1/README.md) passed
Clippy and eight tests, including six fault cases. Its ARM64 driver
retired six and five original guest instructions in the two paths
and produced the expected output words 42 and 0xffffffff. The
architecture audit found only libSystem as a dynamic dependency.
Logs and traces are in `artifacts/x1/`.

The next GPU slice adds VGPR-only logical shifts
`v_lshlrev_b32_e32` and `v_lshrrev_b32_e32`. Both use the low five
bits of the first source as the count for the unsigned second source.
The original combined fixture exercises count wrapping, operand aliases
and high-bit values. The workspace ran 58 tests, the separate LLVM
suite checked 324 three-way comparisons across 18 forms and 24 malformed
or out-of-subset probes, and all 12 native regression tests passed.
The OR and shift fixtures each matched 131 output words on Apple M5
with both validation layers active. Clippy and the native audit passed.
Exit codes, fixture hashes and the working patch are under
`artifacts/phase02-shifts/`.

The isolated [X2 experiment](../../experiments/x1/README.md) loaded
the original fixed-base PE32+ image on ARM64 and retired three guest
instructions. Independent review confirmed that service 1 changed the
argument 41 to 42, resumed at `0x140001007`, and stored the eight-byte
little-endian result. The independent run passed 16 tests, including
alternate arguments, wraparound and rejection before retirement.
Formatting and Clippy passed. The profile rejects relocation and
zero-fill inputs and owns a private map per load. Logs are under
`artifacts/x2-review/`.

X3 passed independent acceptance after replacing the flat fixture with
an original PE32+ image using the existing loader and interpreter.
The independent suite passed 21 tests. Invalid entry and trap edits
rejected the command before dispatch. Five fresh Apple M5 runs with
both Metal validation layers matched the exact three-word output and
zero guest status. Evidence is under `artifacts/x3-pe.pQUnzE/`.

The isolated X4 driver loaded two original PE fixtures and invoked an
8×8 RGBA8 offscreen Metal draw. With API and shader validation enabled
on Apple M5, readback matched all 256 expected bytes per target for
guest parameters 17 and 93. The 24 Metal-feature tests passed.
Implementation evidence is under `artifacts/x4/`. Independent review
confirmed 384 exact pixels over three fresh runs of both fixtures and
all 24 tests passed. Review evidence is under `artifacts/x4-review/`.
