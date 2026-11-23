# SPDX-License-Identifier: GPL-3.0-or-later
"""Generate or verify the original X2 guest image using only Python stdlib."""
import argparse
from pathlib import Path
from pe_fixture import build_pe

parser = argparse.ArgumentParser(__doc__)
parser.add_argument("--check", action="store_true")
args = parser.parse_args()

code = bytes.fromhex("b8 01 00 00 00 0f 0b 48 89 07")  # mov eax, 1; ud2; mov [rdi], rax
data = bytes([0xA5] * 8)
image = build_pe(0x140000000, [
    (b".text", code, 0x60000020),
    (b".data", data, 0xC0000040),
])

fixture = Path(__file__).with_name("x2.exe")
if args.check:
    if fixture.read_bytes() != image:
        parser.exit(1, "X2 fixture mismatch\n")
    print("X2 fixture matches generator")
else:
    fixture.write_bytes(image)
