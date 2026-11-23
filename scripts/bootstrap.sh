#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
set -eu

if [ "$(uname -m)" != arm64 ] || [ "$(arch)" != arm64 ] ||
   [ "$(sysctl -n sysctl.proc_translated)" != 0 ]; then
    printf '%s\n' 'Run on native Apple Silicon macOS.' >&2
    exit 1
fi

rustc -vV
cargo --version
xcodebuild -version
xcrun --find metal
xcrun metal --version

llvm_bin=${LLVM_BIN:-}
if [ -z "$llvm_bin" ] && command -v brew >/dev/null 2>&1 &&
   llvm_prefix=$(brew --prefix llvm 2>/dev/null); then
    llvm_bin="$llvm_prefix/bin"
fi
for tool in llvm-mc llvm-objcopy llvm-objdump; do
    if [ -n "$llvm_bin" ] && [ -x "$llvm_bin/$tool" ]; then
        "$llvm_bin/$tool" --version | head -1
    else
        printf '%s\n' "Optional fixture tooling: set LLVM_BIN to a native LLVM bin directory containing $tool."
    fi
done
