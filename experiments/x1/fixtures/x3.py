# SPDX-License-Identifier: GPL-3.0-or-later
"""Generate or verify the original fixed-base X3 guest bytes."""
import argparse
from pathlib import Path
from struct import pack
from pe_fixture import build_pe

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--check", action="store_true")
args = parser.parse_args()

# mov eax, 0x403000; ud2; mov [rdi], rax
code = bytes.fromhex("b8 00 30 40 00 0f 0b 48 89 07")
# Version, service, kernel, input handle, access, byte offset, byte length, grid words.
command = pack("<8I", 1, 2, 1, 1, 1, 4, 12, 3)
input_buffer = pack("<4I", 0xDEADBEEF, 0, 0x12345678, 0xFFFFFFFF)
image = build_pe(0x400000, [
    (b".text", code, 0x60000020),
    (b".data", bytes([0xA5] * 8), 0xC0000040),
    (b".rdata", command + input_buffer, 0x40000040),
])
fixture = Path(__file__).with_name("x3.exe")
if args.check:
    if fixture.read_bytes() != image:
        parser.exit(1, "X3 fixture mismatch\n")
    print("X3 PE fixture matches generator")
else:
    fixture.write_bytes(image)
