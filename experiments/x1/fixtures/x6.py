#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Build the original X6 fixed-base PE32+ guest fixture.

Entry invokes service 1 with RCX = N, calls a countdown routine using a
backward JMP, then calls a second routine to write service 2's doubled
value to the eight-byte data section. The host supplies the data address in
RDI and starts execution with a fresh stack for every case.
"""
import argparse
from pathlib import Path
from struct import pack_into

# ---------------------------------------------------------------------------

def build_pe(image_base, sections):
    """Build a minimal PE32+ image. sections = list of (name8_bytes, payload_bytes, flags)."""
    n = len(sections)
    file_align = 0x200
    sect_align = 0x1000
    total = file_align * (1 + n)
    image = bytearray(total)

    # DOS header
    image[0:2] = b"MZ"
    pack_into("<I", image, 0x3C, 0x80)  # PE offset

    # COFF header at 0x80
    pe = 0x80
    pack_into("<4s", image, pe, b"PE\0\0")
    pack_into("<H", image, pe + 4, 0x8664)       # machine AMD64
    pack_into("<H", image, pe + 6, n)              # section count
    # timestamp=0, symtable=0, symcount=0
    pack_into("<H", image, pe + 20, 240)           # optional header size
    pack_into("<H", image, pe + 22, 0x0022)        # characteristics (executable)

    # Optional header at pe+24
    opt = pe + 24
    pack_into("<H", image, opt, 0x020B)            # PE32+ magic
    # MajorLinkerVersion, MinorLinkerVersion = 0
    # SizeOfCode
    code_size = sum(len(p) for _, p, f in sections if f & 0x20)
    pack_into("<I", image, opt + 4, code_size)
    idata_size = sum(len(p) for _, p, f in sections if not (f & 0x20))
    pack_into("<I", image, opt + 8, idata_size)
    # SizeOfUninitializedData = 0
    # AddressOfEntryPoint = first section RVA = sect_align
    pack_into("<I", image, opt + 16, sect_align)
    # BaseOfCode
    pack_into("<I", image, opt + 20, sect_align)
    # ImageBase
    pack_into("<Q", image, opt + 24, image_base)
    # SectionAlignment
    pack_into("<I", image, opt + 32, sect_align)
    # FileAlignment
    pack_into("<I", image, opt + 36, file_align)
    # OS/image version = 0
    # SizeOfImage = (1+n) * sect_align
    pack_into("<I", image, opt + 56, sect_align * (1 + n))
    # SizeOfHeaders = file_align
    pack_into("<I", image, opt + 60, file_align)
    # Checksum = 0
    # Subsystem = 3 (console)
    pack_into("<H", image, opt + 68, 3)
    # DllCharacteristics = 0
    # Stack/heap reserve/commit = 0
    # NumberOfRvaAndSizes = 16
    pack_into("<I", image, opt + 108, 16)
    # Data directories: all zero (no imports, no relocations)

    # Section table at pe + 24 + 240 = pe + 264
    table = pe + 264
    for i, (name, payload, flags) in enumerate(sections):
        raw_off = file_align * (1 + i)
        rva = sect_align * (1 + i)
        hdr = table + i * 40
        name8 = name[:8].ljust(8, b"\0")
        pack_into("<8s", image, hdr, name8)
        pack_into("<I", image, hdr + 8, len(payload))   # VirtualSize
        pack_into("<I", image, hdr + 12, rva)           # VirtualAddress
        pack_into("<I", image, hdr + 16, file_align)  # SizeOfRawData
        pack_into("<I", image, hdr + 20, raw_off)       # PointerToRawData
        # PointerToRelocations, PointerToLinenumbers, NumberOf* = 0
        pack_into("<I", image, hdr + 36, flags)         # Characteristics

        # Write raw section data
        image[raw_off:raw_off + len(payload)] = payload

    return bytes(image)


# ---------------------------------------------------------------------------
# X6 code (see layout in docstring above)
# ---------------------------------------------------------------------------

def build_code():
    """Build the 40-byte X6 .text payload byte-by-byte."""
    code = bytearray()

    # entry (offset 0)
    code += bytes.fromhex("b801000000")   # 0: MOV EAX, 1
    code += bytes.fromhex("0f0b")         # 5: UD2 (svc1)
    code += bytes.fromhex("e806000000")   # 7: CALL count_down (next=12, rel=6, target=18)
    code += bytes.fromhex("e80c000000")   # 12: CALL double_store (next=17, rel=12, target=29)
    code += bytes.fromhex("c3")           # 17: RET

    assert len(code) == 18, f"entry section is {len(code)} bytes, expected 18"

    # count_down (offset 18)
    code += bytes.fromhex("83f800")       # 18: CMP EAX, 0
    code += bytes.fromhex("7405")         # 21: JE +5 → 28
    code += bytes.fromhex("83e801")       # 23: SUB EAX, 1
    code += bytes.fromhex("ebf6")         # 26: JMP -10 → 18 (next=28, rel=-10=0xF6)
    code += bytes.fromhex("c3")           # 28: RET

    assert len(code) == 29, f"count_down section ends at {len(code)} bytes, expected 29"

    # double_store (offset 29)
    code += bytes.fromhex("b802000000")   # 29: MOV EAX, 2
    code += bytes.fromhex("0f0b")         # 34: UD2 (svc2)
    code += bytes.fromhex("488907")       # 36: MOV [RDI], RAX
    code += bytes.fromhex("c3")           # 39: RET

    assert len(code) == 40, f"total code is {len(code)} bytes, expected 40"

    # Verify branch targets.
    # JMP at 26: next=28, disp=0xF6=-10 (signed), target=28+(-10)=18. ✓
    assert code[26] == 0xEB, f"JMP opcode expected at 26, got {code[26]:#x}"
    assert code[27] == 0xF6, f"JMP disp expected 0xF6 at 27, got {code[27]:#x}"
    # JE at 21: disp=5, next=23, target=28. ✓
    assert code[21] == 0x74, f"JE opcode expected at 21, got {code[21]:#x}"
    assert code[22] == 0x05, f"JE disp expected 5 at 22, got {code[22]:#x}"

    return bytes(code)


parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
parser.add_argument("--check", action="store_true", help="verify fixture matches; exit 1 on mismatch")
args = parser.parse_args()

code = build_code()
assert len(code) == 40

data = bytes([0xA5] * 8)

image = build_pe(0x140000000, [
    (b".text", code, 0x60000020),   # RX: code + exec + read
    (b".data", data, 0xC0000040),   # RW: init data + read + write
])

fixture = Path(__file__).with_name("x6.exe")
if args.check:
    if not fixture.exists():
        parser.exit(1, "X6 fixture does not exist\n")
    if fixture.read_bytes() != image:
        parser.exit(1, "X6 fixture mismatch\n")
    print("X6 fixture matches generator")
else:
    fixture.write_bytes(image)
    print(f"X6 fixture written to {fixture} ({len(image)} bytes)")
