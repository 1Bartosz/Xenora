#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
set -eu
cd "$(dirname "$0")/../.."
root=$PWD
src="$root/tests/compat_fixtures/import_guest.c"
out="$root/artifacts/compat-fixtures"
clang=${CLANG:-/opt/homebrew/opt/llvm/bin/clang}
dlltool=${LLVM_DLLTOOL:-/opt/homebrew/opt/llvm/bin/llvm-dlltool}
linker=${LLD_LINK:-/opt/homebrew/opt/lld/bin/lld-link}
readobj=${LLVM_READOBJ:-/opt/homebrew/opt/llvm/bin/llvm-readobj}

for tool in "$clang" "$dlltool" "$linker" "$readobj"; do
    case "$(lipo -archs "$tool")" in
        *arm64*) ;;
        *) echo "ARM64 tool required: $tool" >&2; exit 1 ;;
    esac
done

mkdir -p "$out"
cd "$out"
cat > xenora-test.def <<'DEF'
LIBRARY XENORA-TEST.dll
EXPORTS
    xenora_transform
    xenora_report
    xenora_missing
    xenora_compute
    xenora_resource
DEF
"$dlltool" -m i386:x86-64 -d xenora-test.def -l xenora-test.lib
for variant in 0 1; do
    "$clang" --target=x86_64-pc-windows-msvc -O1 -ffreestanding -fno-stack-protector \
        -DUNRESOLVED_IMPORT="$variant" -c "$src" -o "guest-$variant.obj"
    "$linker" /entry:guest_entry /subsystem:console /nodefaultlib /timestamp:0 \
        "/out:guest-$variant.exe" "guest-$variant.obj" xenora-test.lib
    "$readobj" --file-headers --coff-imports "guest-$variant.exe" > "guest-$variant.txt"
    grep -q 'IMAGE_FILE_MACHINE_AMD64' "guest-$variant.txt"
    grep -qx '  Magic: 0x20B' "guest-$variant.txt"
    test "$(sed -n 's/^  Name: //p' "guest-$variant.txt")" = XENORA-TEST.dll
    if [ "$variant" = 0 ]; then
        expected=xenora_report
    else
        expected=xenora_missing
    fi
    actual=$(awk '$1 == "Symbol:" { print $2 }' "guest-$variant.txt" | LC_ALL=C sort)
    test "$actual" = "$(printf '%s\n' "$expected" xenora_transform)"
done
for variant in 0 1 2 3 4 5 6; do
    "$clang" --target=x86_64-pc-windows-msvc -O1 -ffreestanding -fno-stack-protector \
        -DCASE="$variant" -c "$root/tests/compat_fixtures/compute_guest.c" -o "compute-$variant.obj"
    "$linker" /entry:guest_entry /subsystem:console /nodefaultlib /timestamp:0 \
        "/out:compute-$variant.exe" "compute-$variant.obj" xenora-test.lib
done
for variant in 0 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18; do
    "$clang" --target=x86_64-pc-windows-msvc -O1 -ffreestanding -fno-stack-protector -fno-optimize-sibling-calls \
        -DCASE="$variant" -c "$root/tests/compat_fixtures/resource_guest.c" -o "resource-$variant.obj"
    "$linker" /entry:guest_entry /subsystem:console /nodefaultlib /timestamp:0 /fixed \
        "/out:resource-$variant.exe" "resource-$variant.obj" xenora-test.lib
done
for variant in 0 1 2 12 16 18; do
    "$readobj" --file-headers --coff-imports "resource-$variant.exe" > "resource-$variant.txt"
    grep -q 'IMAGE_FILE_MACHINE_AMD64' "resource-$variant.txt"
    grep -qx '  Magic: 0x20B' "resource-$variant.txt"
    test "$(sed -n 's/^  Name: //p' "resource-$variant.txt")" = XENORA-TEST.dll
    actual=$(awk '$1 == "Symbol:" { print $2 }' "resource-$variant.txt" | LC_ALL=C sort)
    test "$actual" = "$(printf '%s\n' xenora_report xenora_resource)"
done
