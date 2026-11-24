# SPDX-License-Identifier: GPL-3.0-or-later
"""Generate the clean-room binary asset fixture for Gate 6 VFS testing."""

import argparse
import math
from pathlib import Path
from struct import pack


def generate_asset():
    data = bytearray()
    # 1. Header: Magic 'XAST' (4B), version 1 (4B), width 2 (4B), height 2 (4B), audio_samples 256 (4B)
    data += b"XAST"
    data += pack("<IIII", 1, 2, 2, 256)

    # 2. Texture data: 2x2 RGBA8 (16 bytes)
    # (0,0): Orange [255, 128, 0, 255]
    # (1,0): Purple [128, 0, 255, 255]
    # (0,1): Teal   [0, 255, 128, 255]
    # (1,1): Pink   [255, 0, 128, 255]
    texture_bytes = bytes([
        255, 128, 0, 255,
        128, 0, 255, 255,
        0, 255, 128, 255,
        255, 0, 128, 255,
    ])
    data += texture_bytes

    # 3. Audio data: 256 samples of 440Hz sine wave (16-bit signed PCM mono, 512 bytes)
    sample_rate = 48000
    freq = 440.0
    for i in range(256):
        sample_val = int(32767.0 * 0.5 * math.sin(2.0 * math.pi * freq * (i / sample_rate)))
        data += pack("<h", sample_val)

    return bytes(data)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    path = Path(__file__).parent / "asset.bin"
    expected = generate_asset()
    if args.check:
        if not path.exists() or path.read_bytes() != expected:
            parser.exit(1, f"asset binary differs from generator: {path}\n")
    else:
        path.write_bytes(expected)
