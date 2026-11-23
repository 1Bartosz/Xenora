# SPDX-License-Identifier: GPL-3.0-or-later
"""Build the fixed-base synthetic guest PE profile."""
from struct import pack_into


def build_pe(image_base, sections):
    image = bytearray(0x200 * (1 + len(sections)))
    image[:2] = b"MZ"
    pack_into("<I", image, 0x3C, 0x80)
    pack_into("<4sHHIIIHH", image, 0x80, b"PE\0\0", 0x8664, len(sections), 0, 0, 0, 240, 0x23)
    optional = 0x98
    pack_into("<HBBIIIIIQII", image, optional,
              0x20B, 0, 0, 0x200, 0x200 * (len(sections) - 1), 0, 0x1000, 0x1000,
              image_base, 0x1000, 0x200)
    pack_into("<H", image, optional + 40, 6)
    pack_into("<H", image, optional + 48, 6)
    pack_into("<IIIHHQQQQII", image, optional + 56,
              0x1000 * (1 + len(sections)), 0x200, 0, 3, 0, 0x100000, 0x1000, 0x100000, 0x1000, 0, 16)
    for index, (name, payload, flags) in enumerate(sections):
        raw = 0x200 * (index + 1)
        rva = 0x1000 * (index + 1)
        pack_into("<8sIIIIIIHHI", image, 0x188 + 40 * index,
                  name, len(payload), rva, 0x200, raw, 0, 0, 0, 0, flags)
        image[raw:raw + len(payload)] = payload

    return image
