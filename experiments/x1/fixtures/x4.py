# SPDX-License-Identifier: GPL-3.0-or-later
"""Generate or verify the two original X4 guest images."""
import argparse
from pathlib import Path
from struct import pack

from pe_fixture import build_pe

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--check", action="store_true")
args = parser.parse_args()

# mov eax, 0x403000; ud2; mov [rdi], rax
code = bytes.fromhex("b8 00 30 40 00 0f 0b 48 89 07")
# Version, service, target handle, width, height, format, parameter offset and length.
command = pack("<8I", 1, 3, 1, 8, 8, 1, 4, 4)
for parameter in (17, 93):
    image = build_pe(0x400000, [
        (b".text", code, 0x60000020),
        (b".data", bytes([0xA5] * 8), 0xC0000040),
        (b".rdata", command + pack("<4I", 0xDEADBEEF, parameter, 0, 0), 0x40000040),
    ])
    fixture = Path(__file__).with_name(f"x4-{parameter}.exe")
    if args.check:
        if fixture.read_bytes() != image:
            parser.exit(1, f"{fixture.name} mismatch\n")
        print(f"{fixture.name} matches generator")
    else:
        fixture.write_bytes(image)
