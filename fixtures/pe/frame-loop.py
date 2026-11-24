# SPDX-License-Identifier: GPL-3.0-or-later
"""Generate the original PE32+ frame-loop fixture without an x86 toolchain.

The guest calls xenora.dll!frame_submit(index) with index = 0, 1, 2, ... and
stops when the import returns 0, then halts. Frame order is guest state (RBX).
"""

import argparse
from pathlib import Path
from struct import pack, pack_into


def fixture():
    base = 0x140000000
    entry, imports = 0x1000, 0x2000
    iat, lookup, module, symbol = (imports + x for x in (0x40, 0x50, 0x60, 0x80))

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
    for index, rva, size in ((1, imports, 40), (12, iat, 16)):
        pack_into("<II", data, opt + 112 + index * 8, rva, size)

    for index, (name, rva, permissions) in enumerate(
        ((b".text", entry, 0x60000020), (b".rdata", imports, 0x40000040))
    ):
        pack_into(
            "<8sIIIIIIHHI", data, opt + 240 + 40 * index, name, 0x200, rva,
            0x200, 0x400 + 0x200 * index, 0, 0, 0, 0, permissions,
        )

    loop = entry + 6
    code = b"\x48\x83\xec\x28"  # sub rsp, 0x28   (Win64 home area)
    code += b"\x31\xdb"  # xor ebx, ebx        (frame index)
    code += b"\x48\x89\xd9"  # loop: mov rcx, rbx
    code += b"\xff\x15" + pack("<i", iat - (loop + 9))  # call [rip -> IAT]
    code += b"\x48\xff\xc3"  # inc rbx
    code += b"\x48\x85\xc0"  # test rax, rax
    code += b"\x75" + pack("<b", loop - (loop + 17))  # jnz loop
    code += b"\x48\x83\xc4\x28\xf4"  # add rsp, 0x28; hlt
    data[0x400:0x400 + len(code)] = code

    pack_into("<IIIII", data, 0x600, lookup, 0, 0, module, iat)
    pack_into("<Q", data, 0x640, symbol)
    pack_into("<Q", data, 0x650, symbol)
    data[0x660:0x66B] = b"xenora.dll\0"
    data[0x682:0x68E] = b"frame_submit"
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
