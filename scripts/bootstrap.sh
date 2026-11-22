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

if command -v brew >/dev/null 2>&1; then
    llvm_bin=${LLVM_BIN:-$(brew --prefix llvm)/bin}
else
    llvm_bin=${LLVM_BIN:-}
fi
for tool in llvm-mc llvm-objcopy llvm-objdump; do
    if [ -n "$llvm_bin" ] && [ -x "$llvm_bin/$tool" ]; then
        "$llvm_bin/$tool" --version | head -1
    else
        printf '%s\n' "Optional fixture tooling: set LLVM_BIN to a native LLVM bin directory containing $tool."
    fi
done
