#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
set -eu
cd "$(dirname "$0")/.."

# Source mutation campaign for the critical Phase 01 modules.
# R2 owns execution and survivor triage; this script only reproduces the run.
# Tests run for the mutated package only (cargo-mutants default), so this stays
# CPU-only and never dispatches GPU work.
out=${1:-artifacts/mutation/campaign-$(date +%Y%m%d-%H%M%S)}
mkdir -p "$out"
export CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS:-2}

cargo mutants --workspace \
    --file 'crates/decoder/src/**' \
    --file 'crates/shader/src/**' \
    --file 'crates/ref/src/**' \
    --jobs 2 \
    --timeout 60 \
    --output "$out"
