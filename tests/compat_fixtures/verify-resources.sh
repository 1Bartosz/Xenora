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

cargo build --quiet --locked --manifest-path experiments/x1/Cargo.toml \
    --target aarch64-apple-darwin --features metal
driver="$CARGO_TARGET_DIR/aarch64-apple-darwin/debug/xenora-x1"
test "$(lipo -archs "$driver")" = arm64

run_guest() {
    timeout 15s "$driver" x9 "$out/resource-$1.exe"
}

python3 - "$out" <<'PY'
import struct
import sys
from fractions import Fraction
from pathlib import Path

output = Path(sys.argv[1])

def expect(case, reports, sizes):
    # The report import reads three words. The last BC1 report contains an
    # unchanged guard that is deliberately excluded from the native readback.
    assert len(reports) % 3 == 0
    lines = [
        f"X9 xenora_report: {reports[start:start + 3]}"
        for start in range(0, len(reports), 3)
    ]
    cursor = 0
    for words in sizes:
        readback = struct.pack(f"<{words}I", *reports[cursor:cursor + words])
        lines.append(f"X9 readback: {readback.hex()}")
        cursor += words
    lines.append(f"X9 guest exit status: {sum(reports[-3:]) % 2**32}")
    (output / f"resource-expected-{case}.log").write_text("\n".join(lines) + "\n")

def pixels(rgba):
    return [int.from_bytes(bytes(pixel), "little") for pixel in rgba]

xor = [word ^ 0x01020304 for word in (2, 5, 11, 17, 19, 23, 17, 19, 23, 27, 31, 41)]
expect(0, xor, [3, 3, 3, 3])
expect(1, pixels([(255, 0, 0, 255)] * 12), [12])

# At each pixel center the NDC position lies within the full coverage
# triangle. With W=1, the green vertex weight is (x+1)/4 in NDC and the
# red vertex weight is (y+1)/4. Convert to bytes without GPU inputs.
varying = [
    (
        round(255 * (1 - Fraction(2 * row + 1, 6)) / 2),
        round(255 * Fraction(2 * col + 1, 16)),
        255,
        255,
    )
    for row in range(3)
    for col in range(4)
]
expect(2, pixels(varying), [12])
expect(12, pixels(([(0, 255, 0, 255)] * 4 + [(0, 0, 255, 255)]) * 3), [15])
expect(16, [0xffffffff, 0xffffffff, 0x11223344], [2])
PY

for variant in 0 1 2 12 16; do
    for repeat in 1 2; do
        log="$out/resource-pass-$variant-$repeat"
        run_guest "$variant" > "$log.log" 2> "$log.err"
        grep -q "Metal API Validation Enabled" "$log.err"
        grep -q "Metal GPU Validation Enabled" "$log.err"
        diff -u "$out/resource-expected-$variant.log" "$log.log"
    done
    cat "$out/resource-pass-$variant-1.log"
done
# A deliberately wrong interior pixel expectation must change the guest
# result to 93 while the native render bytes still match the varying draw.
log="$out/resource-wrong-expected"
run_guest 18 > "$log.log" 2> "$log.err"
grep -q "Metal API Validation Enabled" "$log.err"
grep -q "Metal GPU Validation Enabled" "$log.err"
{
    sed -n '1,3p; /^X9 readback:/p' "$out/resource-expected-2.log"
    printf '%s\n' 'X9 guest exit status: 93'
} > "$out/resource-expected-18.log"
diff -u "$out/resource-expected-18.log" "$log.log"

for variant in 3 4 5 6 7 8 10 11 13 14 15 17; do
    case "$variant" in
        3) fault='StaleResource' ;;
        4) fault='InvalidResource' ;;
        7) fault='MemFault(NotWritable)' ;;
        8) fault='SubmissionPending' ;;
        11) fault='NoSubmission' ;;
        *) fault='InvalidResourceView' ;;
    esac
    status=0
    log="$out/resource-fail-$variant"
    run_guest "$variant" > "$log.log" 2> "$log.err" || status=$?
    test "$status" -eq 1
    test ! -s "$log.log"
    grep -F "guest fault $fault at RIP" "$log.err"
    if grep -Eq 'Metal (API|GPU) Validation Enabled' "$log.err"; then
        exit 1
    fi
done
run_guest 9 > "$out/resource-abandon.log" 2> "$out/resource-abandon.err"
test "$(cat "$out/resource-abandon.log")" = 'X9 guest exit status: 73'
test ! -s "$out/resource-abandon.err"
