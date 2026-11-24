#!/usr/bin/env python3
"""Assemble the original x86-64 integer fixture with the LLVM assembler.

  generate   write crates/loader/tests/x86/integer.txt
  --check    reassemble in memory and fail if the committed fixture differs

Each output line is `<hex bytes> | <Intel-syntax text>`. The assembler is
`clang -target x86_64-apple-macos` and the bytes are read with `objdump -d`,
so no Xenora code takes part in choosing an encoding. Nothing is executed.
"""

import re
import subprocess
import sys
import tempfile
from pathlib import Path

OUT = Path(__file__).resolve().parent / "integer.txt"
LOW = "a c d b sp bp si di".split()
CONDITIONS = "o no b ae e ne be a s ns p np l ge le g".split()
PTR = {8: "byte", 16: "word", 32: "dword", 64: "qword"}


def reg(index, bits):
    if index >= 8:
        return f"r{index}" + {64: "", 32: "d", 16: "w", 8: "b"}[bits]
    name = LOW[index]
    if bits == 8:
        return name + "l"
    full = name + "x" if index < 4 else name
    return {64: "r", 32: "e", 16: ""}[bits] + full


def mem(bits):
    return f"{PTR[bits]} ptr [rsi + 4*rdi + 16]"


def cases():
    out = []
    for op in "add or adc sbb and sub xor cmp test".split():
        for bits in (8, 16, 32, 64):
            out += [f"{op} {reg(3, bits)}, {reg(1, bits)}", f"{op} {reg(9, bits)}, {reg(12, bits)}",
                    f"{op} {reg(0, bits)}, {reg(0, bits)}", f"{op} {mem(bits)}, {reg(2, bits)}"]
            if op != "test":
                # The reversed encoding of a register pair is load-form only.
                out.append(f"{op} {reg(10, bits)}, {mem(bits)}")
            imm = {8: "0x7f", 16: "0x1234", 32: "0x12345678", 64: "0x12345678"}[bits]
            out += [f"{op} {reg(0, bits)}, {imm}", f"{op} {reg(3, bits)}, {imm}",
                    f"{op} {mem(bits)}, {imm}"]
            if bits != 8:
                out += [f"{op} {reg(11, bits)}, -3", f"{op} {mem(bits)}, -3"]
        out += [f"{op} ah, bl", f"{op} cl, dh", f"{op} bh, 0x55"]
    for op in "inc dec neg not mul imul div idiv".split():
        for bits in (8, 16, 32, 64):
            out += [f"{op} {reg(1, bits)}", f"{op} {reg(9, bits)}", f"{op} {mem(bits)}"]
        out.append(f"{op} ch")
    for op in "rol ror shl shr sar".split():
        for bits in (8, 16, 32, 64):
            for count in ("1", "3", "7", "cl"):
                out += [f"{op} {reg(3, bits)}, {count}", f"{op} {mem(bits)}, {count}"]
            out += [f"{op} {reg(13, bits)}, {bits - 1}", f"{op} {reg(13, bits)}, 0"]
    for bits in (16, 32, 64):
        out += [f"imul {reg(3, bits)}, {reg(9, bits)}", f"imul {reg(3, bits)}, {mem(bits)}",
                f"imul {reg(8, bits)}, {reg(1, bits)}, -7", f"imul {reg(8, bits)}, {reg(1, bits)}, 0x1234",
                f"imul {reg(8, bits)}, {mem(bits)}, 0x1234"]
        for op in "bsf bsr popcnt tzcnt lzcnt".split():
            out += [f"{op} {reg(3, bits)}, {reg(9, bits)}", f"{op} {reg(10, bits)}, {mem(bits)}"]
        for op in "bt bts btr btc".split():
            out += [f"{op} {reg(3, bits)}, {reg(9, bits)}", f"{op} {reg(10, bits)}, 5",
                    f"{op} {mem(bits)}, {bits - 1}"]
        for cc in CONDITIONS:
            out += [f"cmov{cc} {reg(3, bits)}, {reg(9, bits)}", f"cmov{cc} {reg(10, bits)}, {mem(bits)}"]
        for src in (8, 16):
            if src < bits:
                for op in ("movzx", "movsx"):
                    out += [f"{op} {reg(3, bits)}, {reg(9, src)}", f"{op} {reg(10, bits)}, {mem(src)}"]
        out += [f"lea {reg(3, bits)}, [rsi + 4*rdi + 16]", f"lea {reg(12, bits)}, [r9 - 5]",
                f"lea {reg(0, bits)}, [8*r10 + 64]"]
    out += ["movsxd rbx, r9d", f"movsxd r10, {mem(32)}", "movzx ebx, ah", "movsx ecx, bh"]
    for cc in CONDITIONS:
        out += [f"set{cc} bl", f"set{cc} {mem(8)}", f"set{cc} ah"]
    for bits in (8, 16, 32, 64):
        out += [f"mov {reg(3, bits)}, {reg(9, bits)}", f"mov {reg(10, bits)}, {mem(bits)}",
                f"mov {mem(bits)}, {reg(2, bits)}", f"mov {reg(14, bits)}, 0x5a",
                f"mov {mem(bits)}, 0x5a", f"xchg {reg(3, bits)}, {reg(9, bits)}",
                f"xchg {mem(bits)}, {reg(2, bits)}"]
    out += ["mov ah, bl", "mov dl, ch", "mov bh, 0x21", "xchg ah, bl", "mov sil, dil",
            "movabs r15, 0x8877665544332211", "mov rcx, -2", "bswap ebx", "bswap r9",
            "cbw", "cwde", "cdqe", "cwd", "cdq", "cqo", "clc", "stc", "cmc", "nop",
            "nop dword ptr [rax + 8]", "pause", "push r9", "push rbx", "push -5", "push 0x12345678",
            f"push {mem(64)}", "pop rbx", "pop r12", f"pop {mem(64)}", "leave"]
    return list(dict.fromkeys(out))


def assemble(lines):
    with tempfile.TemporaryDirectory() as directory:
        source, obj = Path(directory, "a.s"), Path(directory, "a.o")
        source.write_text(".intel_syntax noprefix\n" + "\n".join(lines) + "\n")
        subprocess.run(["clang", "-target", "x86_64-apple-macos", "-c", source, "-o", obj], check=True)
        # One line per instruction: objdump never wraps its raw bytes.
        listing = subprocess.run(["objdump", "-d", obj], check=True, capture_output=True, text=True).stdout
    rows = [m.group(1).split() for m in re.finditer(r"^\s*[0-9a-f]+:\s((?:[0-9a-f]{2}\s)+)", listing, re.M)]
    if len(rows) != len(lines):
        sys.exit(f"{len(lines)} instructions assembled to {len(rows)} rows")
    return "".join(f"{' '.join(row)} | {line}\n" for row, line in zip(rows, lines))


def main(argv):
    if len(argv) != 2 or argv[1] not in ("generate", "--check"):
        sys.exit("usage: assemble.py generate|--check")
    text = assemble(cases())
    if argv[1] == "generate":
        OUT.write_text(text)
    elif OUT.read_text() != text:
        sys.exit("integer.txt differs from the assembler output")
    print(f"{text.count(chr(10))} instructions")


if __name__ == "__main__":
    main(sys.argv)
