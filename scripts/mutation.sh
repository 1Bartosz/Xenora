#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
set -eu
cd "$(dirname "$0")/.."

# Source mutation campaign for the critical Phase 01 modules.
# R2 owns execution and survivor triage; this script only reproduces the run.
# Run the default workspace tests so downstream CPU validation can catch
# mutations. Native GPU tests and the LLVM oracle are opt-in.
out=${1:-artifacts/mutation/campaign-$(date +%Y%m%d-%H%M%S)}
mkdir -p "$out"
export CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS:-2}

cargo mutants --workspace \
    --file 'crates/decoder/src/**' \
    --file 'crates/shader/src/**' \
    --file 'crates/ref/src/**' \
    --test-workspace true \
    --jobs 2 \
    --timeout 300 \
    --output "$out"
