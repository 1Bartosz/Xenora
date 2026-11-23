# Synthetic PE32+ import fixtures

Run `tests/compat_fixtures/build.sh` on ARM64 macOS with native LLVM and LLD.
The script writes two x86-64 guest executables and their import listings under
`artifacts/compat-fixtures/`. Both variants come from the original C source
and are linked without the C runtime. The generated import library supplies
linker metadata for a synthetic DLL.

## Expected runtime behavior

The synthetic guest service resolver binds these imports from `XENORA-TEST.dll`.

- `xenora_transform` receives a writable guest range of three unsigned
  32-bit words. It doubles each word and returns the number written.
  The input `[2, 5, 11]` must become `[4, 10, 22]`.
- `xenora_report` reads three unsigned 32-bit words and returns their sum.
  The expected guest exit status is 36. Exit status 91 indicates that the
  transform did not return the expected word count.
- The second executable imports `xenora_missing` in place of
  `xenora_report`. The resolver must reject it before entering the guest.

The build script checks import metadata using LLVM. Run
`tests/compat_fixtures/verify.sh` to build both images, execute the positive
guest, compare the service trace and exit value, and check pre-entry rejection
of the unresolved symbol. Relocation and thread-local storage initialization
are outside this fixture's scope.

## Guest compute acceptance

Run `tests/compat_fixtures/verify-compute.sh` on native Apple Silicon with
LLVM and Metal. It builds the original `compute_guest.c` and exercises the
named-import runner with the Metal feature. The positive guest submits a
three-word resource and reports the Metal readback twice in separate runs.
The gate checks the guest's returned sum and the Metal validation banners.
The negative variants cover the command, handle, binding, count, unmapped
buffer and stale one-shot handle.

## Persistent resources and translated drawing

Run `tests/compat_fixtures/verify-resources.sh` on native Apple Silicon
with LLVM, Python and Metal. It builds the source-compiled
`resource_guest.c` PE32+ variants, runs each positive fixture twice and
checks the exact native readback, guest memory report and guest exit status.
Each run has a 15-second timeout. Metal API and shader validation must be
enabled.

- Compute submits a three-word view after a guard word. It updates the live
  buffer before completion, dispatches again, and queues work that completes
  from the old snapshot after release and slot reuse.
- Draw submits an original full-coverage triangle to the translated
  vertex and pixel stages in a 4×3 viewport. The guest directly loads and
  compares all 12 pixel words for each of the constant and interpolated
  draws. The verification script separately derives the interpolated
  values at the pixel centers. A control changes the guest's expected
  interior pixel and checks that the guest returns 93 even though the same
  native draw bytes are returned.
- BC1 selects the base or smaller mip from the second array layer of a
  pitched upload and reads the native texture through an authored Metal
  kernel. The smaller mip checks that the guest guard word after the
  output is preserved.
- Invalid and stale handles, bounds, offset overflow, a read-only output,
  an occupied or empty submission slot, malformed draw input and
  malformed or read-only texture operations fail before Metal execution.
  The abandoned submission returns 73 without executing the queued work.

The runner prints the guest's exit value and returns a nonzero host status
for load or interpreter faults. The gate compares both outcomes. These
fixtures exercise synthetic commands and local Apple Silicon readback.
