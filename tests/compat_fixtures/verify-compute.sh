#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
set -eu
cd "$(dirname "$0")/../.."
out="$PWD/artifacts/compat-fixtures"

tests/compat_fixtures/build.sh
export CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS:-2}
export CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-"$PWD/artifacts/build"}
export MTL_DEBUG_LAYER=1 MTL_SHADER_VALIDATION=1
export MTL_SHADER_VALIDATION_REPORT_TO_STDERR=1 MTL_SHADER_VALIDATION_ABORT_ON_FAULT=1

run_guest() {
    cargo run --quiet --locked --manifest-path experiments/x1/Cargo.toml \
        --target aarch64-apple-darwin --features metal -- x7 "$out/compute-$1.exe"
}

# Independently calculated XOR with 0x01020304 of 2, 5, and 11, then the sum.
cat > "$out/compute-expected.log" <<'EXPECTED'
X7 xenora_compute: [16909062, 16909057, 16909071]
X7 xenora_report: [16909062, 16909057, 16909071]
X7 guest exit status: 50727190
EXPECTED
for repeat in 1 2; do
    run_guest 0 > "$out/compute-pass-$repeat.log" 2> "$out/compute-pass-$repeat.err"
    grep -q "Metal API Validation Enabled" "$out/compute-pass-$repeat.err"
    grep -q "Metal GPU Validation Enabled" "$out/compute-pass-$repeat.err"
    diff -u "$out/compute-expected.log" "$out/compute-pass-$repeat.log"
done
for variant in 1 2 3 4 5 6; do
    case "$variant" in
        5) fault='MemFault(Unmapped)' ;;
        6) fault='StaleResource' ;;
        *) fault='InvalidServiceArgument' ;;
    esac
    status=0
    run_guest "$variant" > "$out/compute-fail-$variant.log" \
        2> "$out/compute-fail-$variant.err" || status=$?
    test "$status" -eq 1
    test ! -s "$out/compute-fail-$variant.log"
    grep -F "guest fault $fault at RIP" "$out/compute-fail-$variant.err"
    if [ "$variant" -lt 6 ]; then
        ! grep -q 'Metal API Validation Enabled' "$out/compute-fail-$variant.err" || exit 1
    else
        grep -q 'Metal GPU Validation Enabled' "$out/compute-fail-$variant.err"
    fi
done
cat "$out/compute-pass-1.log"
