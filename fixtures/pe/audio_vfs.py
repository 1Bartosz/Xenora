# SPDX-License-Identifier: GPL-3.0-or-later
"""Generate the clean-room PE32+ audio and virtual filesystem fixture without an x86 toolchain.

The guest demonstrates virtual asset loading and PCM audio streaming through runtime services:
1. Calls xenora.dll!vfs_open_asset() to open the authorized asset 'asset.bin'.
2. Calls xenora.dll!vfs_read_asset(handle, 548) to load asset header, texture, and PCM audio.
3. Calls xenora.dll!vfs_close_asset(handle) to close the asset file.
4. Calls xenora.dll!audio_init(48000, 1, 1) to initialize audio (48kHz, mono, S16LE).
5. Calls xenora.dll!audio_submit_pcm(512) to submit the 256 PCM audio samples.
6. Calls xenora.dll!frame_submit_textured(0) to render the 2x2 loaded texture on Metal.
7. Halts cleanly.
"""

import argparse
from pathlib import Path
from struct import pack, pack_into


def fixture():
    base = 0x140000000
    entry, imports = 0x1000, 0x2000
    lookup = imports + 0x30
    iat = imports + 0x70
    module = imports + 0xB0
    sym_vfs_open = imports + 0xC0
    sym_vfs_read = imports + 0xE0
    sym_vfs_close = imports + 0x100
    sym_audio_init = imports + 0x120
    sym_audio_submit = imports + 0x140
    sym_frame_submit = imports + 0x160

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
    for index, rva, size in ((1, imports, 40), (12, iat, 56)):
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

    # 1. vfs_open_asset() -> rax = file handle
    # call [rip -> iat[0]: vfs_open_asset]
    code += b"\xff\x15" + pack("<i", iat - (entry + len(code) + 6))
    code += b"\x48\x89\xc3"      # mov rbx, rax (save file handle)

    # 2. vfs_read_asset(handle, 548) -> rax = bytes read
    code += b"\x48\x89\xd9"      # mov rcx, rbx (handle)
    code += b"\xba\x24\x02\x00\x00"  # mov edx, 548 (header 20B + tex 16B + audio 512B)
    # call [rip -> iat[1]: vfs_read_asset]
    code += b"\xff\x15" + pack("<i", (iat + 8) - (entry + len(code) + 6))

    # 3. vfs_close_asset(handle)
    code += b"\x48\x89\xd9"      # mov rcx, rbx (handle)
    # call [rip -> iat[2]: vfs_close_asset]
    code += b"\xff\x15" + pack("<i", (iat + 16) - (entry + len(code) + 6))

    # 4. audio_init(48000, 1, 1)
    code += b"\xb9\x80\xbb\x00\x00"  # mov ecx, 48000 (sample rate)
    code += b"\xba\x01\x00\x00\x00"  # mov edx, 1 (mono)
    code += b"\x41\xb8\x01\x00\x00\x00"  # mov r8d, 1 (S16LE)
    # call [rip -> iat[3]: audio_init]
    code += b"\xff\x15" + pack("<i", (iat + 24) - (entry + len(code) + 6))

    # 5. audio_submit_pcm(512)
    code += b"\xb9\x00\x02\x00\x00"  # mov ecx, 512 (bytes)
    # call [rip -> iat[4]: audio_submit_pcm]
    code += b"\xff\x15" + pack("<i", (iat + 32) - (entry + len(code) + 6))

    # 6. frame_submit_textured(0)
    code += b"\x31\xc9"          # xor ecx, ecx
    # call [rip -> iat[5]: frame_submit_textured]
    code += b"\xff\x15" + pack("<i", (iat + 40) - (entry + len(code) + 6))

    code += b"\x48\x83\xc4\x28"  # add rsp, 0x28
    code += b"\xf4"              # hlt

    data[0x400:0x400 + len(code)] = code

    # Import directory table
    pack_into("<IIIII", data, 0x600, lookup, 0, 0, module, iat)
    # Lookups & IAT (6 imports)
    for off in (lookup - imports, iat - imports):
        pack_into("<QQQQQQ", data, 0x600 + off,
                  sym_vfs_open, sym_vfs_read, sym_vfs_close,
                  sym_audio_init, sym_audio_submit, sym_frame_submit)

    data[0x600 + (module - imports):0x600 + (module - imports) + 11] = b"xenora.dll\0"
    data[0x600 + (sym_vfs_open - imports) + 2:0x600 + (sym_vfs_open - imports) + 2 + 14] = b"vfs_open_asset"
    data[0x600 + (sym_vfs_read - imports) + 2:0x600 + (sym_vfs_read - imports) + 2 + 14] = b"vfs_read_asset"
    data[0x600 + (sym_vfs_close - imports) + 2:0x600 + (sym_vfs_close - imports) + 2 + 15] = b"vfs_close_asset"
    data[0x600 + (sym_audio_init - imports) + 2:0x600 + (sym_audio_init - imports) + 2 + 10] = b"audio_init"
    data[0x600 + (sym_audio_submit - imports) + 2:0x600 + (sym_audio_submit - imports) + 2 + 16] = b"audio_submit_pcm"
    data[0x600 + (sym_frame_submit - imports) + 2:0x600 + (sym_frame_submit - imports) + 2 + 21] = b"frame_submit_textured"
    return data


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    path = Path(__file__).with_suffix(".exe")
    expected = fixture()
    if args.check:
        if not path.exists() or path.read_bytes() != expected:
            parser.exit(1, f"fixture bytes differ from generator: {path}\n")
    else:
        path.write_bytes(expected)
