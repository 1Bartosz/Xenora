# SPDX-License-Identifier: GPL-3.0-or-later
"""Generate the original PE32+ interactive fixture without an x86 toolchain.

The guest polls host input via xenora.dll!input_poll() and updates its
internal position:
- input 1 (Right): column = (column + 1) % 4
- input 2 (Left): column = (column - 1) % 4
- other: column stays unchanged

The guest then calls xenora.dll!frame_submit(column) and stops when the
import returns 0, then halts.
"""

import argparse
from pathlib import Path
from struct import pack, pack_into


def fixture():
    base = 0x140000000
    entry, imports = 0x1000, 0x2000
    lookup = imports + 0x30
    iat = imports + 0x50
    module = imports + 0x70
    sym_poll = imports + 0x80
    sym_submit = imports + 0x90

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
    for index, rva, size in ((1, imports, 40), (12, iat, 24)):
        pack_into("<II", data, opt + 112 + index * 8, rva, size)

    for index, (name, rva, permissions) in enumerate(
        ((b".text", entry, 0x60000020), (b".rdata", imports, 0x40000040))
    ):
        pack_into(
            "<8sIIIIIIHHI", data, opt + 240 + 40 * index, name, 0x200, rva,
            0x200, 0x400 + 0x200 * index, 0, 0, 0, 0, permissions,
        )

    code = bytearray()
    code += b"\x48\x83\xec\x28"  # sub rsp, 0x28
    code += b"\x48\x31\xdb"      # xor rbx, rbx (column: 0)

    loop_start = entry + len(code)
    # call [rip -> iat[0]: input_poll]
    code += b"\xff\x15" + pack("<i", iat - (entry + len(code) + 6))

    # cmp rax, 1; jne not_right; add rbx, 1
    code += b"\x48\x83\xf8\x01\x75\x04\x48\x83\xc3\x01"
    # cmp rax, 2; jne not_left; sub rbx, 1
    code += b"\x48\x83\xf8\x02\x75\x04\x48\x83\xeb\x01"
    # wrap check: cmp rbx, 4; jl check_min; sub rbx, 4
    code += b"\x48\x83\xfb\x04\x7c\x04\x48\x83\xeb\x04"
    # check_min: cmp rbx, 0; jge do_submit; add rbx, 4
    code += b"\x48\x83\xfb\x00\x7d\x04\x48\x83\xc3\x04"

    # do_submit:
    code += b"\x48\x89\xd9"      # mov rcx, rbx
    # call [rip -> iat[1]: frame_submit]
    code += b"\xff\x15" + pack("<i", (iat + 8) - (entry + len(code) + 6))

    code += b"\x48\x85\xc0"      # test rax, rax
    rel_loop = loop_start - (entry + len(code) + 2)
    code += b"\x75" + pack("<b", rel_loop)

    code += b"\x48\x83\xc4\x28"  # add rsp, 0x28
    code += b"\xf4"              # hlt

    data[0x400:0x400 + len(code)] = code

    pack_into("<IIIII", data, 0x600, lookup, 0, 0, module, iat)
    pack_into("<QQ", data, 0x600 + (lookup - imports), sym_poll, sym_submit)
    pack_into("<QQ", data, 0x600 + (iat - imports), sym_poll, sym_submit)
    data[0x600 + (module - imports):0x600 + (module - imports) + 11] = b"xenora.dll\0"
    data[0x600 + (sym_poll - imports) + 2:0x600 + (sym_poll - imports) + 2 + 10] = b"input_poll"
    data[0x600 + (sym_submit - imports) + 2:0x600 + (sym_submit - imports) + 2 + 12] = b"frame_submit"
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
