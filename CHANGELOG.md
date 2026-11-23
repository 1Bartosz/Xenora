# Changelog

Notable changes are grouped using [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
Experimental interfaces may change. Authorized releases should identify their
exact revision and known limitations.

## [Unreleased]

Guest execution uses original, purpose-built test programs. Commercial Xbox game execution
remains unsupported.

### Added

- Added a public documentation index for architecture, validation, and contributor references.

- Added a bounded x86-64 guest interpreter on native ARM64 macOS (X1), plus a loader for original fixed-base 64-bit Portable Executable (PE32+) fixtures and an experimental add-one service (X2).
- Added guest-to-GPU command submission for a validated synthetic compute workload (X3) and guest-driven Metal rendering to an offscreen 8×8 RGBA8 texture (X4). Independent review confirmed the compute output and every pixel for both rendering inputs on Apple M5.
- Added a typed shader intermediate representation and Metal Shading Language generation for a bounded subset of 32-lane AMD RDNA 2 compute on the desktop gfx1030 target, with fixture validation on Apple M5. Expanded the verified vector-register-only forms to include bitwise OR and logical shifts. Arithmetic right shift passed local CPU and Metal checks, with independent acceptance still open.
- Added independent CPU known-answer fixtures, LLVM decoder comparisons, and source and native semantic mutation checks.
- Added a correctness-gated benchmark harness that records stage timings, power source, and thermal state.
- Repository contribution guidelines, issue forms, pull request checklist, community expectations and security reporting policy.

### Changed

- Added native ARM64 CI and repository audits for dependency direction, claims, and generated status. Expanded CI to check the standalone guest experiment, synthetic PE fixture bytes, and progress asset consistency.
- Reworked the README around demonstrated capabilities and added a deterministic SVG drawn from the reviewed form inventory and guest acceptance records.
- Made reading and updating the changelog mandatory for engineering work.

### Fixed

- Validate branch targets, buffer bindings, and program termination throughout a shader fixture, including instructions skipped by a resolved branch.
- Include arithmetic right shift in fixture reassembly checks and let bootstrap continue when optional LLVM discovery fails.

### Removed

- Removed the superseded Phase 01 continuation prompt and redundant planning instructions. Candidate verification records remain in the assurance documentation.

- Removed the speculative seven-phase roadmap from the README.
