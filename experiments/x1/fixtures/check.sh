#!/bin/bash
# SPDX-License-Identifier: GPL-3.0-or-later
set -euo pipefail

fixtures="$(cd "$(dirname "$0")" && pwd)"
: "${LLVM_BIN:?Set LLVM_BIN to a native ARM64 LLVM bin directory}"
scratch="$(mktemp -d)"
trap 'rm -rf "$scratch"' EXIT

for tool in "$LLVM_BIN/llvm-mc" "$LLVM_BIN/llvm-objcopy"; do
    file "$tool"
    lipo -archs "$tool" | tee "$scratch/arch"
    if ! grep -qw arm64 "$scratch/arch"; then
        exit 1
    fi
    otool -L "$tool"
    arch -arm64 "$tool" --version
done

for case in wrap nonwrap; do
    initial_eax=0xffffffff
    if [[ "$case" == nonwrap ]]; then
        initial_eax=0xfffffffe
    fi
    {
        printf '.set initial_eax, %s\n' "$initial_eax"
        cat "$fixtures/x1.s"
    } | arch -arm64 "$LLVM_BIN/llvm-mc" -triple=x86_64-unknown-linux-gnu \
        -filetype=obj -o "$scratch/$case.o"
    arch -arm64 "$LLVM_BIN/llvm-objcopy" -O binary --only-section=.text \
        "$scratch/$case.o" "$scratch/$case.bin"
    cmp "$fixtures/$case.bin" "$scratch/$case.bin"
    od -An -v -tx1 "$fixtures/$case.bin"
    shasum -a 256 "$fixtures/$case.bin"
done
