#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Build the guest C structured-exception program,
# tests/compat_fixtures/guest_c_seh.c (#453), as freestanding x86-64 PE32+
# executables: one per case and optimisation level, named
# guest-c-seh-<case>-<level>.exe.
#
#   build-guest-c-seh.sh [output directory]   default: artifacts/guest-c-seh
#
# Tools, native to the host:
#   CLANG     a clang with the x86_64-pc-windows-msvc target. Default:
#             Homebrew LLVM if installed, otherwise the one on PATH.
#   LLD_LINK  an lld-link. Default: Homebrew lld if installed, otherwise the
#             one on PATH.
#
# -fasync-exceptions makes clang treat a CPU fault inside __try as catchable;
# without it no handler covers the faulting instruction. The program has no C
# runtime: its imports come from import libraries made below from definition
# files, and nothing from a Windows SDK is used.
set -eu
cd "$(dirname "$0")/../.."
root=$PWD
src="$root/tests/compat_fixtures/guest_c_seh.c"
out=${1:-"$root/artifacts/guest-c-seh"}

clang=${CLANG:-}
if [ -z "$clang" ]; then
    clang=/opt/homebrew/opt/llvm/bin/clang
    [ -x "$clang" ] || clang=$(command -v clang || true)
fi
linker=${LLD_LINK:-}
if [ -z "$linker" ]; then
    linker=/opt/homebrew/opt/lld/bin/lld-link
    [ -x "$linker" ] || linker=$(command -v lld-link || true)
fi
for tool in "$clang" "$linker"; do
    if [ -z "$tool" ] || [ ! -x "$tool" ]; then
        echo "missing tool: clang and lld-link are required (set CLANG and LLD_LINK)" >&2
        exit 1
    fi
done

# AGENTS.md: host tools must be ARM64, checked with file, lipo -archs and
# otool -L.
if [ "$(uname -s)" = Darwin ]; then
    for tool in "$clang" "$linker"; do
        case "$(file -bL "$tool")" in
            *arm64*) ;;
            *) echo "ARM64 tool required (file): $tool" >&2; exit 1 ;;
        esac
        case "$(lipo -archs "$tool")" in
            *arm64*) ;;
            *) echo "ARM64 tool required (lipo): $tool" >&2; exit 1 ;;
        esac
        for library in $(otool -L "$tool" | awk 'NR > 1 { print $1 }'); do
            [ -f "$library" ] || continue
            case "$(lipo -archs "$library")" in
                *arm64*) ;;
                *) echo "ARM64 library required by $tool: $library" >&2; exit 1 ;;
            esac
        done
    done
fi

mkdir -p "$out"
cd "$out"
{
    "$clang" --version | head -1
    "$linker" --version | head -1
} > toolchain.txt

# Import libraries for the four imported services. xenora_seh.dll is not a
# real module: the test registers its one service, xenora_note.
cat > kernel32.def <<'DEF'
LIBRARY kernel32.dll
EXPORTS
    ExitProcess
    RaiseException
DEF
cat > ucrtbase.def <<'DEF'
LIBRARY ucrtbase.dll
EXPORTS
    __C_specific_handler
DEF
cat > xenora_seh.def <<'DEF'
LIBRARY xenora_seh.dll
EXPORTS
    xenora_note
DEF
for module in kernel32 ucrtbase xenora_seh; do
    "$linker" /lib /machine:x64 "/def:$module.def" "/out:$module.lib" > /dev/null
done

for level in O0 O1 O2; do
    for case in divide search unhandled write_fault read_fault execute_fault \
        illegal finally_across_frames finally_same_frame continue_execution; do
        name="guest-c-seh-$case-$level"
        "$clang" --target=x86_64-pc-windows-msvc "-$level" -fasync-exceptions \
            -ffreestanding -fno-stack-protector -Wall -Wextra -Werror \
            -Wno-unused-function "-DCASE=$case" -c "$src" -o "$name.obj"
        "$linker" /entry:entry /subsystem:console /nodefaultlib /machine:x64 \
            "/out:$name.exe" "$name.obj" kernel32.lib ucrtbase.lib xenora_seh.lib \
            > /dev/null
    done
done
