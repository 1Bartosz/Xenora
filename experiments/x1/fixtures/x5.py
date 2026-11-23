# SPDX-License-Identifier: GPL-3.0-or-later
"""Generate or verify the original X5 guest PE32+ image using only Python stdlib.

X5 layout (30 bytes of code):
  [0..5]  b8 01 00 00 00  MOV EAX,1          load service-1 ID
  [5..7]  0f 0b           UD2                 service 1: identity, sets EAX = RCX (N)
  [7..10] 83 f8 00        CMP EAX,0           is N zero?
  [10..12] 74 06          JE  +6  → [18]      if zero, skip loop body
  [12..15] 83 e8 01       SUB EAX,1           N--
  [15..18] 83 f8 00       CMP EAX,0           is decremented value zero?
  [18..20] 75 f3          JNE -13 → [7]       if not zero, loop back to CMP
  [20..25] b8 02 00 00 00 MOV EAX,2          load service-2 ID
  [25..27] 0f 0b           UD2                service 2: EAX = RCX*2
  [27..30] 48 89 07        STORE [RDI]        write qword EAX to guest data section

Branch verification (instruction boundaries):
  JE  next=12, disp=+6, target=18:   0→5→7→10→12→15→18 ✓  (JNE at 18)
  JNE next=20, disp=-13, target=7:   0→5→7 ✓             (CMP at 7)

Independent expected outputs (derived before code was run):
  N=0 → service1→0, JE taken→[18], JNE falls through, service2(rcx=0)→0  → data=[0,0,0,0,0,0,0,0]
  N=1 → service1→1, JE falls, SUB→0, JNE falls, service2(rcx=1)→2       → data=[2,0,0,0,0,0,0,0]
  N=2 → service1→2, 1 loop iteration, service2(rcx=2)→4                  → data=[4,0,0,0,0,0,0,0]
  N=3 → service1→3, 2 loop iterations, service2(rcx=3)→6                 → data=[6,0,0,0,0,0,0,0]
"""
import argparse
from pathlib import Path
from pe_fixture import build_pe

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--check", action="store_true", help="verify fixture matches; exit 1 on mismatch")
args = parser.parse_args()

code = bytes.fromhex(
    "b801000000"  # MOV EAX,1
    "0f0b"        # UD2  (service 1: identity → EAX=N)
    "83f800"      # CMP EAX,0
    "7406"        # JE +6 → [18]
    "83e801"      # SUB EAX,1
    "83f800"      # CMP EAX,0
    "75f3"        # JNE -13 → [7]
    "b802000000"  # MOV EAX,2
    "0f0b"        # UD2  (service 2: double → EAX=RCX*2)
    "488907"      # STORE [RDI]
)
assert len(code) == 30, f"expected 30 code bytes, got {len(code)}"

# Sentinel data to detect zero-fill and partial-write bugs.
data = bytes([0xA5] * 8)

image = build_pe(0x140000000, [
    (b".text", code, 0x60000020),
    (b".data", data, 0xC0000040),
])

fixture = Path(__file__).with_name("x5.exe")
if args.check:
    if not fixture.exists():
        parser.exit(1, "X5 fixture does not exist\n")
    if fixture.read_bytes() != image:
        parser.exit(1, "X5 fixture mismatch\n")
    print("X5 fixture matches generator")
else:
    fixture.write_bytes(image)
    print(f"X5 fixture written to {fixture} ({len(image)} bytes)")
