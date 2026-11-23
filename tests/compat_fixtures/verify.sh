#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
set -eu
cd "$(dirname "$0")/../.."
root=$PWD
out="$root/artifacts/compat-fixtures"

tests/compat_fixtures/build.sh
export CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS:-2}
export CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-"$out/target"}

run_guest() {
    cargo run --quiet --locked --manifest-path experiments/x1/Cargo.toml \
        --target aarch64-apple-darwin -- x7 "$1"
}

run_guest "$out/guest-0.exe" > "$out/positive.log"
cat > "$out/expected.log" <<'EXPECTED'
X7 xenora_transform: [4, 10, 22]
X7 xenora_report: [4, 10, 22]
X7 guest exit status: 36
EXPECTED
diff -u "$out/expected.log" "$out/positive.log"

negative_status=0
run_guest "$out/guest-1.exe" > "$out/negative.log" 2> "$out/negative-error.log" ||
    negative_status=$?
test "$negative_status" -eq 1
test ! -s "$out/negative.log"
grep -qx 'X7: unresolved import XENORA-TEST.dll!xenora_missing' "$out/negative-error.log"
cat "$out/positive.log" "$out/negative-error.log"
