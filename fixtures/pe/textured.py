# SPDX-License-Identifier: GPL-3.0-or-later
"""Generate the original PE32+ persistent texture fixture without an x86 toolchain.

The guest demonstrates persistent GPU resource ownership, dynamic texture updates,
and completion-aware resource retirement through runtime services:
1. Calls xenora.dll!texture_create(2, 2) to allocate a persistent 2x2 texture.
2. Calls xenora.dll!frame_submit_textured(handle) to render frame 0 with initial contents.
3. Calls xenora.dll!texture_update(handle, 0, 0, 2, 2) to dynamically update texture pixels.
4. Calls xenora.dll!frame_submit_textured(handle) to render frame 1 with updated contents.
5. Calls xenora.dll!texture_destroy(handle) to retire the resource.
6. Halts cleanly.
"""

import argparse
from pathlib import Path
from struct import pack, pack_into


def fixture():
    base = 0x140000000
    entry, imports = 0x1000, 0x2000
    lookup = imports + 0x30
    iat = imports + 0x60
    module = imports + 0x90
    sym_create = imports + 0xA0
    sym_update = imports + 0xC0
    sym_submit = imports + 0xE0
    sym_destroy = imports + 0x100

    data = bytearray(0x800)
    data[:2] = b"MZ"
    pack_into("<I", data, 0x3C, 0x80)
    data[0x80:0x84] = b"PE\0\0"
    pack_into("<HHIIIHH", data, 0x84, 0x8664, 2, 0, 0, 0, 240, 0x22)
    opt = 0x98
    pack_into("<H", data, opt, 0x20B)
    pack_into("<III", data, opt + 4, 0x200, 0x200, 0)
    pack_into("<IIQII", data, opt + 16, entry, entry, base, 0x1000, 0x200)
    pack_into("<II", data, opt + 56, 0x3000, 0x400)
    pack_into("<HH", data, opt + 68, 3, 0x160)
    pack_into("<QQQQII", data, opt + 72, 0x10000, 0x1000, 0x10000, 0x1000, 0, 16)
    for index, rva, size in ((1, imports, 40), (12, iat, 40)):
        pack_into("<II", data, opt + 112 + index * 8, rva, size)

    for index, (name, rva, permissions) in enumerate(
        ((b".text", entry, 0x60000020), (b".rdata", imports, 0x40000040))
    ):
        pack_into(
            "<8sIIIIIIHHI", data, opt + 240 + 40 * index, name, 0x200, rva,
            0x200, 0x400 + 0x200 * index, 0, 0, 0, 0, permissions,
        )

    code = bytearray()
    code += b"\x48\x83\xec\x28"  # sub rsp, 0x28 (Win64 home area)

    # 1. texture_create(2, 2)
    code += b"\x48\xc7\xc1\x02\x00\x00\x00"  # mov rcx, 2
    code += b"\x48\xc7\xc2\x02\x00\x00\x00"  # mov rdx, 2
    # call [rip -> iat[0]: texture_create]
    code += b"\xff\x15" + pack("<i", iat - (entry + len(code) + 6))
    code += b"\x48\x89\xc3"                  # mov rbx, rax (save handle)

    # 2. frame_submit_textured(handle)
    code += b"\x48\x89\xd9"                  # mov rcx, rbx
    # call [rip -> iat[2]: frame_submit_textured]
    code += b"\xff\x15" + pack("<i", (iat + 16) - (entry + len(code) + 6))

    # 3. texture_update(handle, 0, 0, 2, 2)
    code += b"\x48\x89\xd9"                  # mov rcx, rbx
    code += b"\x48\x31\xd2"                  # xor rdx, rdx (x=0)
    code += b"\x49\xc7\xc0\x00\x00\x00\x00"  # mov r8, 0    (y=0)
    code += b"\x49\xc7\xc1\x02\x00\x00\x00"  # mov r9, 2    (w=2)
    # call [rip -> iat[1]: texture_update]
    code += b"\xff\x15" + pack("<i", (iat + 8) - (entry + len(code) + 6))

    # 4. frame_submit_textured(handle)
    code += b"\x48\x89\xd9"                  # mov rcx, rbx
    # call [rip -> iat[2]: frame_submit_textured]
    code += b"\xff\x15" + pack("<i", (iat + 16) - (entry + len(code) + 6))

    # 5. texture_destroy(handle)
    code += b"\x48\x89\xd9"                  # mov rcx, rbx
    # call [rip -> iat[3]: texture_destroy]
    code += b"\xff\x15" + pack("<i", (iat + 24) - (entry + len(code) + 6))

    code += b"\x48\x83\xc4\x28"              # add rsp, 0x28
    code += b"\xf4"                          # hlt

    data[0x400:0x400 + len(code)] = code

    # Import directory table
    pack_into("<IIIII", data, 0x600, lookup, 0, 0, module, iat)
    # Lookups & IAT
    for off in (lookup - imports, iat - imports):
        pack_into("<QQQQ", data, 0x600 + off, sym_create, sym_update, sym_submit, sym_destroy)

    data[0x600 + (module - imports):0x600 + (module - imports) + 11] = b"xenora.dll\0"
    data[0x600 + (sym_create - imports) + 2:0x600 + (sym_create - imports) + 2 + 14] = b"texture_create"
    data[0x600 + (sym_update - imports) + 2:0x600 + (sym_update - imports) + 2 + 14] = b"texture_update"
    data[0x600 + (sym_submit - imports) + 2:0x600 + (sym_submit - imports) + 2 + 21] = b"frame_submit_textured"
    data[0x600 + (sym_destroy - imports) + 2:0x600 + (sym_destroy - imports) + 2 + 15] = b"texture_destroy"
    return data


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    path = Path(__file__).with_suffix(".exe")
    if args.check:
        if path.read_bytes() != fixture():
            parser.exit(1, f"fixture bytes differ from generator: {path}\n")
    else:
        path.write_bytes(fixture())
