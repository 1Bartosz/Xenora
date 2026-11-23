# SPDX-License-Identifier: GPL-3.0-or-later
.intel_syntax noprefix
.text
    mov eax, initial_eax
    add eax, 1
    cmp eax, 0
    jne store
    mov eax, 42
store:
    mov [rdi], rax
