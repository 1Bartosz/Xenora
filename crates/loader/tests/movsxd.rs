// SPDX-License-Identifier: GPL-3.0-or-later

//! MOVZX, MOVSX and MOVSXD known-answer tests (issues #65, #66 and #117), and
//! the MOV-immediate regressions that keep one-byte opcodes B8-BF separate from
//! the two-byte 0F BE/BF sign extensions.
//!
//! Every encoding was produced from its Intel-syntax text by the LLVM assembler
//! (`clang -target x86_64-apple-macos -c` then `objdump -d`, Apple clang 17.0.0).
//! Expected values follow the Intel SDM Vol. 2 operation sections and are
//! computed here by modular arithmetic, independently of both engines.

use xenora_loader::cpu::{CpuState, ExecutionResult, ExitReason, Interpreter, RFlags, Register};
use xenora_loader::dbt::DbtEngine;
use xenora_loader::memory::{GuestMemory, MemoryPermissions};

const ENTRY: u64 = 0x1000;
const DATA: u64 = 0x2000;
const DATA_LEN: usize = 0x400;

struct Case {
    text: &'static str,
    code: &'static [u8],
    signed: bool,
    dst: usize,
    dst_bits: u32,
    /// Register number, or for `high` the parent of AH/CH/DH/BH.
    src: usize,
    src_bits: u32,
    high: bool,
}

#[rustfmt::skip]
const CASES: &[Case] = &[
    Case { text: "movsx ax, cl", code: &[0x66, 0x0F, 0xBE, 0xC1], signed: true, dst: 0, dst_bits: 16, src: 1, src_bits: 8, high: false },
    Case { text: "movsx bx, al", code: &[0x66, 0x0F, 0xBE, 0xD8], signed: true, dst: 3, dst_bits: 16, src: 0, src_bits: 8, high: false },
    Case { text: "movsx r8w, r9b", code: &[0x66, 0x45, 0x0F, 0xBE, 0xC1], signed: true, dst: 8, dst_bits: 16, src: 9, src_bits: 8, high: false },
    Case { text: "movsx cx, r12b", code: &[0x66, 0x41, 0x0F, 0xBE, 0xCC], signed: true, dst: 1, dst_bits: 16, src: 12, src_bits: 8, high: false },
    Case { text: "movsx r10w, dl", code: &[0x66, 0x44, 0x0F, 0xBE, 0xD2], signed: true, dst: 10, dst_bits: 16, src: 2, src_bits: 8, high: false },
    Case { text: "movsx si, dil", code: &[0x66, 0x40, 0x0F, 0xBE, 0xF7], signed: true, dst: 6, dst_bits: 16, src: 7, src_bits: 8, high: false },
    Case { text: "movsx dx, dl", code: &[0x66, 0x0F, 0xBE, 0xD2], signed: true, dst: 2, dst_bits: 16, src: 2, src_bits: 8, high: false },
    Case { text: "movsx eax, cl", code: &[0x0F, 0xBE, 0xC1], signed: true, dst: 0, dst_bits: 32, src: 1, src_bits: 8, high: false },
    Case { text: "movsx ebx, al", code: &[0x0F, 0xBE, 0xD8], signed: true, dst: 3, dst_bits: 32, src: 0, src_bits: 8, high: false },
    Case { text: "movsx r8d, r9b", code: &[0x45, 0x0F, 0xBE, 0xC1], signed: true, dst: 8, dst_bits: 32, src: 9, src_bits: 8, high: false },
    Case { text: "movsx ecx, r12b", code: &[0x41, 0x0F, 0xBE, 0xCC], signed: true, dst: 1, dst_bits: 32, src: 12, src_bits: 8, high: false },
    Case { text: "movsx r10d, dl", code: &[0x44, 0x0F, 0xBE, 0xD2], signed: true, dst: 10, dst_bits: 32, src: 2, src_bits: 8, high: false },
    Case { text: "movsx esi, dil", code: &[0x40, 0x0F, 0xBE, 0xF7], signed: true, dst: 6, dst_bits: 32, src: 7, src_bits: 8, high: false },
    Case { text: "movsx edx, dl", code: &[0x0F, 0xBE, 0xD2], signed: true, dst: 2, dst_bits: 32, src: 2, src_bits: 8, high: false },
    Case { text: "movsx rax, cl", code: &[0x48, 0x0F, 0xBE, 0xC1], signed: true, dst: 0, dst_bits: 64, src: 1, src_bits: 8, high: false },
    Case { text: "movsx rbx, al", code: &[0x48, 0x0F, 0xBE, 0xD8], signed: true, dst: 3, dst_bits: 64, src: 0, src_bits: 8, high: false },
    Case { text: "movsx r8, r9b", code: &[0x4D, 0x0F, 0xBE, 0xC1], signed: true, dst: 8, dst_bits: 64, src: 9, src_bits: 8, high: false },
    Case { text: "movsx rcx, r12b", code: &[0x49, 0x0F, 0xBE, 0xCC], signed: true, dst: 1, dst_bits: 64, src: 12, src_bits: 8, high: false },
    Case { text: "movsx r10, dl", code: &[0x4C, 0x0F, 0xBE, 0xD2], signed: true, dst: 10, dst_bits: 64, src: 2, src_bits: 8, high: false },
    Case { text: "movsx rsi, dil", code: &[0x48, 0x0F, 0xBE, 0xF7], signed: true, dst: 6, dst_bits: 64, src: 7, src_bits: 8, high: false },
    Case { text: "movsx rdx, dl", code: &[0x48, 0x0F, 0xBE, 0xD2], signed: true, dst: 2, dst_bits: 64, src: 2, src_bits: 8, high: false },
    Case { text: "movsx eax, cx", code: &[0x0F, 0xBF, 0xC1], signed: true, dst: 0, dst_bits: 32, src: 1, src_bits: 16, high: false },
    Case { text: "movsx ebx, ax", code: &[0x0F, 0xBF, 0xD8], signed: true, dst: 3, dst_bits: 32, src: 0, src_bits: 16, high: false },
    Case { text: "movsx r8d, r9w", code: &[0x45, 0x0F, 0xBF, 0xC1], signed: true, dst: 8, dst_bits: 32, src: 9, src_bits: 16, high: false },
    Case { text: "movsx ecx, r12w", code: &[0x41, 0x0F, 0xBF, 0xCC], signed: true, dst: 1, dst_bits: 32, src: 12, src_bits: 16, high: false },
    Case { text: "movsx r10d, dx", code: &[0x44, 0x0F, 0xBF, 0xD2], signed: true, dst: 10, dst_bits: 32, src: 2, src_bits: 16, high: false },
    Case { text: "movsx esi, bp", code: &[0x0F, 0xBF, 0xF5], signed: true, dst: 6, dst_bits: 32, src: 5, src_bits: 16, high: false },
    Case { text: "movsx edx, dx", code: &[0x0F, 0xBF, 0xD2], signed: true, dst: 2, dst_bits: 32, src: 2, src_bits: 16, high: false },
    Case { text: "movsx rax, cx", code: &[0x48, 0x0F, 0xBF, 0xC1], signed: true, dst: 0, dst_bits: 64, src: 1, src_bits: 16, high: false },
    Case { text: "movsx rbx, ax", code: &[0x48, 0x0F, 0xBF, 0xD8], signed: true, dst: 3, dst_bits: 64, src: 0, src_bits: 16, high: false },
    Case { text: "movsx r8, r9w", code: &[0x4D, 0x0F, 0xBF, 0xC1], signed: true, dst: 8, dst_bits: 64, src: 9, src_bits: 16, high: false },
    Case { text: "movsx rcx, r12w", code: &[0x49, 0x0F, 0xBF, 0xCC], signed: true, dst: 1, dst_bits: 64, src: 12, src_bits: 16, high: false },
    Case { text: "movsx r10, dx", code: &[0x4C, 0x0F, 0xBF, 0xD2], signed: true, dst: 10, dst_bits: 64, src: 2, src_bits: 16, high: false },
    Case { text: "movsx rsi, bp", code: &[0x48, 0x0F, 0xBF, 0xF5], signed: true, dst: 6, dst_bits: 64, src: 5, src_bits: 16, high: false },
    Case { text: "movsx rdx, dx", code: &[0x48, 0x0F, 0xBF, 0xD2], signed: true, dst: 2, dst_bits: 64, src: 2, src_bits: 16, high: false },
    Case { text: "movsx dx, ah", code: &[0x66, 0x0F, 0xBE, 0xD4], signed: true, dst: 2, dst_bits: 16, src: 0, src_bits: 8, high: true },
    Case { text: "movsx dx, ch", code: &[0x66, 0x0F, 0xBE, 0xD5], signed: true, dst: 2, dst_bits: 16, src: 1, src_bits: 8, high: true },
    Case { text: "movsx dx, dh", code: &[0x66, 0x0F, 0xBE, 0xD6], signed: true, dst: 2, dst_bits: 16, src: 2, src_bits: 8, high: true },
    Case { text: "movsx dx, bh", code: &[0x66, 0x0F, 0xBE, 0xD7], signed: true, dst: 2, dst_bits: 16, src: 3, src_bits: 8, high: true },
    Case { text: "movsx edx, ah", code: &[0x0F, 0xBE, 0xD4], signed: true, dst: 2, dst_bits: 32, src: 0, src_bits: 8, high: true },
    Case { text: "movsx edx, ch", code: &[0x0F, 0xBE, 0xD5], signed: true, dst: 2, dst_bits: 32, src: 1, src_bits: 8, high: true },
    Case { text: "movsx edx, dh", code: &[0x0F, 0xBE, 0xD6], signed: true, dst: 2, dst_bits: 32, src: 2, src_bits: 8, high: true },
    Case { text: "movsx edx, bh", code: &[0x0F, 0xBE, 0xD7], signed: true, dst: 2, dst_bits: 32, src: 3, src_bits: 8, high: true },
    Case { text: "movzx ax, cl", code: &[0x66, 0x0F, 0xB6, 0xC1], signed: false, dst: 0, dst_bits: 16, src: 1, src_bits: 8, high: false },
    Case { text: "movzx bx, al", code: &[0x66, 0x0F, 0xB6, 0xD8], signed: false, dst: 3, dst_bits: 16, src: 0, src_bits: 8, high: false },
    Case { text: "movzx r8w, r9b", code: &[0x66, 0x45, 0x0F, 0xB6, 0xC1], signed: false, dst: 8, dst_bits: 16, src: 9, src_bits: 8, high: false },
    Case { text: "movzx cx, r12b", code: &[0x66, 0x41, 0x0F, 0xB6, 0xCC], signed: false, dst: 1, dst_bits: 16, src: 12, src_bits: 8, high: false },
    Case { text: "movzx r10w, dl", code: &[0x66, 0x44, 0x0F, 0xB6, 0xD2], signed: false, dst: 10, dst_bits: 16, src: 2, src_bits: 8, high: false },
    Case { text: "movzx si, dil", code: &[0x66, 0x40, 0x0F, 0xB6, 0xF7], signed: false, dst: 6, dst_bits: 16, src: 7, src_bits: 8, high: false },
    Case { text: "movzx dx, dl", code: &[0x66, 0x0F, 0xB6, 0xD2], signed: false, dst: 2, dst_bits: 16, src: 2, src_bits: 8, high: false },
    Case { text: "movzx eax, cl", code: &[0x0F, 0xB6, 0xC1], signed: false, dst: 0, dst_bits: 32, src: 1, src_bits: 8, high: false },
    Case { text: "movzx ebx, al", code: &[0x0F, 0xB6, 0xD8], signed: false, dst: 3, dst_bits: 32, src: 0, src_bits: 8, high: false },
    Case { text: "movzx r8d, r9b", code: &[0x45, 0x0F, 0xB6, 0xC1], signed: false, dst: 8, dst_bits: 32, src: 9, src_bits: 8, high: false },
    Case { text: "movzx ecx, r12b", code: &[0x41, 0x0F, 0xB6, 0xCC], signed: false, dst: 1, dst_bits: 32, src: 12, src_bits: 8, high: false },
    Case { text: "movzx r10d, dl", code: &[0x44, 0x0F, 0xB6, 0xD2], signed: false, dst: 10, dst_bits: 32, src: 2, src_bits: 8, high: false },
    Case { text: "movzx esi, dil", code: &[0x40, 0x0F, 0xB6, 0xF7], signed: false, dst: 6, dst_bits: 32, src: 7, src_bits: 8, high: false },
    Case { text: "movzx edx, dl", code: &[0x0F, 0xB6, 0xD2], signed: false, dst: 2, dst_bits: 32, src: 2, src_bits: 8, high: false },
    Case { text: "movzx rax, cl", code: &[0x48, 0x0F, 0xB6, 0xC1], signed: false, dst: 0, dst_bits: 64, src: 1, src_bits: 8, high: false },
    Case { text: "movzx rbx, al", code: &[0x48, 0x0F, 0xB6, 0xD8], signed: false, dst: 3, dst_bits: 64, src: 0, src_bits: 8, high: false },
    Case { text: "movzx r8, r9b", code: &[0x4D, 0x0F, 0xB6, 0xC1], signed: false, dst: 8, dst_bits: 64, src: 9, src_bits: 8, high: false },
    Case { text: "movzx rcx, r12b", code: &[0x49, 0x0F, 0xB6, 0xCC], signed: false, dst: 1, dst_bits: 64, src: 12, src_bits: 8, high: false },
    Case { text: "movzx r10, dl", code: &[0x4C, 0x0F, 0xB6, 0xD2], signed: false, dst: 10, dst_bits: 64, src: 2, src_bits: 8, high: false },
    Case { text: "movzx rsi, dil", code: &[0x48, 0x0F, 0xB6, 0xF7], signed: false, dst: 6, dst_bits: 64, src: 7, src_bits: 8, high: false },
    Case { text: "movzx rdx, dl", code: &[0x48, 0x0F, 0xB6, 0xD2], signed: false, dst: 2, dst_bits: 64, src: 2, src_bits: 8, high: false },
    Case { text: "movzx eax, cx", code: &[0x0F, 0xB7, 0xC1], signed: false, dst: 0, dst_bits: 32, src: 1, src_bits: 16, high: false },
    Case { text: "movzx ebx, ax", code: &[0x0F, 0xB7, 0xD8], signed: false, dst: 3, dst_bits: 32, src: 0, src_bits: 16, high: false },
    Case { text: "movzx r8d, r9w", code: &[0x45, 0x0F, 0xB7, 0xC1], signed: false, dst: 8, dst_bits: 32, src: 9, src_bits: 16, high: false },
    Case { text: "movzx ecx, r12w", code: &[0x41, 0x0F, 0xB7, 0xCC], signed: false, dst: 1, dst_bits: 32, src: 12, src_bits: 16, high: false },
    Case { text: "movzx r10d, dx", code: &[0x44, 0x0F, 0xB7, 0xD2], signed: false, dst: 10, dst_bits: 32, src: 2, src_bits: 16, high: false },
    Case { text: "movzx esi, bp", code: &[0x0F, 0xB7, 0xF5], signed: false, dst: 6, dst_bits: 32, src: 5, src_bits: 16, high: false },
    Case { text: "movzx edx, dx", code: &[0x0F, 0xB7, 0xD2], signed: false, dst: 2, dst_bits: 32, src: 2, src_bits: 16, high: false },
    Case { text: "movzx rax, cx", code: &[0x48, 0x0F, 0xB7, 0xC1], signed: false, dst: 0, dst_bits: 64, src: 1, src_bits: 16, high: false },
    Case { text: "movzx rbx, ax", code: &[0x48, 0x0F, 0xB7, 0xD8], signed: false, dst: 3, dst_bits: 64, src: 0, src_bits: 16, high: false },
    Case { text: "movzx r8, r9w", code: &[0x4D, 0x0F, 0xB7, 0xC1], signed: false, dst: 8, dst_bits: 64, src: 9, src_bits: 16, high: false },
    Case { text: "movzx rcx, r12w", code: &[0x49, 0x0F, 0xB7, 0xCC], signed: false, dst: 1, dst_bits: 64, src: 12, src_bits: 16, high: false },
    Case { text: "movzx r10, dx", code: &[0x4C, 0x0F, 0xB7, 0xD2], signed: false, dst: 10, dst_bits: 64, src: 2, src_bits: 16, high: false },
    Case { text: "movzx rsi, bp", code: &[0x48, 0x0F, 0xB7, 0xF5], signed: false, dst: 6, dst_bits: 64, src: 5, src_bits: 16, high: false },
    Case { text: "movzx rdx, dx", code: &[0x48, 0x0F, 0xB7, 0xD2], signed: false, dst: 2, dst_bits: 64, src: 2, src_bits: 16, high: false },
    Case { text: "movzx dx, ah", code: &[0x66, 0x0F, 0xB6, 0xD4], signed: false, dst: 2, dst_bits: 16, src: 0, src_bits: 8, high: true },
    Case { text: "movzx dx, ch", code: &[0x66, 0x0F, 0xB6, 0xD5], signed: false, dst: 2, dst_bits: 16, src: 1, src_bits: 8, high: true },
    Case { text: "movzx dx, dh", code: &[0x66, 0x0F, 0xB6, 0xD6], signed: false, dst: 2, dst_bits: 16, src: 2, src_bits: 8, high: true },
    Case { text: "movzx dx, bh", code: &[0x66, 0x0F, 0xB6, 0xD7], signed: false, dst: 2, dst_bits: 16, src: 3, src_bits: 8, high: true },
    Case { text: "movzx edx, ah", code: &[0x0F, 0xB6, 0xD4], signed: false, dst: 2, dst_bits: 32, src: 0, src_bits: 8, high: true },
    Case { text: "movzx edx, ch", code: &[0x0F, 0xB6, 0xD5], signed: false, dst: 2, dst_bits: 32, src: 1, src_bits: 8, high: true },
    Case { text: "movzx edx, dh", code: &[0x0F, 0xB6, 0xD6], signed: false, dst: 2, dst_bits: 32, src: 2, src_bits: 8, high: true },
    Case { text: "movzx edx, bh", code: &[0x0F, 0xB6, 0xD7], signed: false, dst: 2, dst_bits: 32, src: 3, src_bits: 8, high: true },
    Case { text: "movsxd rax, ecx", code: &[0x48, 0x63, 0xC1], signed: true, dst: 0, dst_bits: 64, src: 1, src_bits: 32, high: false },
    Case { text: "movsxd rbx, eax", code: &[0x48, 0x63, 0xD8], signed: true, dst: 3, dst_bits: 64, src: 0, src_bits: 32, high: false },
    Case { text: "movsxd r8, r9d", code: &[0x4D, 0x63, 0xC1], signed: true, dst: 8, dst_bits: 64, src: 9, src_bits: 32, high: false },
    Case { text: "movsxd rcx, r12d", code: &[0x49, 0x63, 0xCC], signed: true, dst: 1, dst_bits: 64, src: 12, src_bits: 32, high: false },
    Case { text: "movsxd r10, edx", code: &[0x4C, 0x63, 0xD2], signed: true, dst: 10, dst_bits: 64, src: 2, src_bits: 32, high: false },
    Case { text: "movsxd rsi, ebp", code: &[0x48, 0x63, 0xF5], signed: true, dst: 6, dst_bits: 64, src: 5, src_bits: 32, high: false },
    Case { text: "movsxd rdx, edx", code: &[0x48, 0x63, 0xD2], signed: true, dst: 2, dst_bits: 64, src: 2, src_bits: 32, high: false },
];

#[rustfmt::skip]
const MOV_IMMEDIATE: &[(&[u8], usize, u64)] = &[
    (&[0xB8, 0x81, 0xBE, 0xF0, 0x8B], 0, 0x8BF0_BE81), // mov eax, 0x8bf0be81
    (&[0xB9, 0x81, 0xBE, 0xF0, 0x8B], 1, 0x8BF0_BE81), // mov ecx, 0x8bf0be81
    (&[0xBA, 0x81, 0xBE, 0xF0, 0x8B], 2, 0x8BF0_BE81), // mov edx, 0x8bf0be81
    (&[0xBB, 0x81, 0xBE, 0xF0, 0x8B], 3, 0x8BF0_BE81), // mov ebx, 0x8bf0be81
    (&[0xBC, 0x81, 0xBE, 0xF0, 0x8B], 4, 0x8BF0_BE81), // mov esp, 0x8bf0be81
    (&[0xBD, 0x81, 0xBE, 0xF0, 0x8B], 5, 0x8BF0_BE81), // mov ebp, 0x8bf0be81
    (&[0xBE, 0x81, 0xBE, 0xF0, 0x8B], 6, 0x8BF0_BE81), // mov esi, 0x8bf0be81
    (&[0xBF, 0x81, 0xBE, 0xF0, 0x8B], 7, 0x8BF0_BE81), // mov edi, 0x8bf0be81
    (&[0x41, 0xB8, 0x81, 0xBE, 0xF0, 0x8B], 8, 0x8BF0_BE81), // mov r8d, 0x8bf0be81
    (&[0x41, 0xB9, 0x81, 0xBE, 0xF0, 0x8B], 9, 0x8BF0_BE81), // mov r9d, 0x8bf0be81
    (&[0x41, 0xBA, 0x81, 0xBE, 0xF0, 0x8B], 10, 0x8BF0_BE81), // mov r10d, 0x8bf0be81
    (&[0x41, 0xBB, 0x81, 0xBE, 0xF0, 0x8B], 11, 0x8BF0_BE81), // mov r11d, 0x8bf0be81
    (&[0x41, 0xBC, 0x81, 0xBE, 0xF0, 0x8B], 12, 0x8BF0_BE81), // mov r12d, 0x8bf0be81
    (&[0x41, 0xBD, 0x81, 0xBE, 0xF0, 0x8B], 13, 0x8BF0_BE81), // mov r13d, 0x8bf0be81
    (&[0x41, 0xBE, 0x81, 0xBE, 0xF0, 0x8B], 14, 0x8BF0_BE81), // mov r14d, 0x8bf0be81
    (&[0x41, 0xBF, 0x81, 0xBE, 0xF0, 0x8B], 15, 0x8BF0_BE81), // mov r15d, 0x8bf0be81
    (&[0x48, 0xB8, 0xF1, 0xB8, 0xC1, 0x63, 0xBF, 0x0F, 0xBE, 0x80], 0, 0x80BE_0FBF_63C1_B8F1), // movabs rax, 0x80be0fbf63c1b8f1
    (&[0x48, 0xB9, 0xF1, 0xB8, 0xC1, 0x63, 0xBF, 0x0F, 0xBE, 0x80], 1, 0x80BE_0FBF_63C1_B8F1), // movabs rcx, 0x80be0fbf63c1b8f1
    (&[0x48, 0xBA, 0xF1, 0xB8, 0xC1, 0x63, 0xBF, 0x0F, 0xBE, 0x80], 2, 0x80BE_0FBF_63C1_B8F1), // movabs rdx, 0x80be0fbf63c1b8f1
    (&[0x48, 0xBB, 0xF1, 0xB8, 0xC1, 0x63, 0xBF, 0x0F, 0xBE, 0x80], 3, 0x80BE_0FBF_63C1_B8F1), // movabs rbx, 0x80be0fbf63c1b8f1
    (&[0x48, 0xBC, 0xF1, 0xB8, 0xC1, 0x63, 0xBF, 0x0F, 0xBE, 0x80], 4, 0x80BE_0FBF_63C1_B8F1), // movabs rsp, 0x80be0fbf63c1b8f1
    (&[0x48, 0xBD, 0xF1, 0xB8, 0xC1, 0x63, 0xBF, 0x0F, 0xBE, 0x80], 5, 0x80BE_0FBF_63C1_B8F1), // movabs rbp, 0x80be0fbf63c1b8f1
    (&[0x48, 0xBE, 0xF1, 0xB8, 0xC1, 0x63, 0xBF, 0x0F, 0xBE, 0x80], 6, 0x80BE_0FBF_63C1_B8F1), // movabs rsi, 0x80be0fbf63c1b8f1
    (&[0x48, 0xBF, 0xF1, 0xB8, 0xC1, 0x63, 0xBF, 0x0F, 0xBE, 0x80], 7, 0x80BE_0FBF_63C1_B8F1), // movabs rdi, 0x80be0fbf63c1b8f1
    (&[0x49, 0xB8, 0xF1, 0xB8, 0xC1, 0x63, 0xBF, 0x0F, 0xBE, 0x80], 8, 0x80BE_0FBF_63C1_B8F1), // movabs r8, 0x80be0fbf63c1b8f1
    (&[0x49, 0xB9, 0xF1, 0xB8, 0xC1, 0x63, 0xBF, 0x0F, 0xBE, 0x80], 9, 0x80BE_0FBF_63C1_B8F1), // movabs r9, 0x80be0fbf63c1b8f1
    (&[0x49, 0xBA, 0xF1, 0xB8, 0xC1, 0x63, 0xBF, 0x0F, 0xBE, 0x80], 10, 0x80BE_0FBF_63C1_B8F1), // movabs r10, 0x80be0fbf63c1b8f1
    (&[0x49, 0xBB, 0xF1, 0xB8, 0xC1, 0x63, 0xBF, 0x0F, 0xBE, 0x80], 11, 0x80BE_0FBF_63C1_B8F1), // movabs r11, 0x80be0fbf63c1b8f1
    (&[0x49, 0xBC, 0xF1, 0xB8, 0xC1, 0x63, 0xBF, 0x0F, 0xBE, 0x80], 12, 0x80BE_0FBF_63C1_B8F1), // movabs r12, 0x80be0fbf63c1b8f1
    (&[0x49, 0xBD, 0xF1, 0xB8, 0xC1, 0x63, 0xBF, 0x0F, 0xBE, 0x80], 13, 0x80BE_0FBF_63C1_B8F1), // movabs r13, 0x80be0fbf63c1b8f1
    (&[0x49, 0xBE, 0xF1, 0xB8, 0xC1, 0x63, 0xBF, 0x0F, 0xBE, 0x80], 14, 0x80BE_0FBF_63C1_B8F1), // movabs r14, 0x80be0fbf63c1b8f1
    (&[0x49, 0xBF, 0xF1, 0xB8, 0xC1, 0x63, 0xBF, 0x0F, 0xBE, 0x80], 15, 0x80BE_0FBF_63C1_B8F1), // movabs r15, 0x80be0fbf63c1b8f1
];

fn mask(bits: u32) -> u128 {
    (1u128 << bits) - 1
}

/// Architectural destination value, derived with modular arithmetic.
fn expected(case: &Case, gpr: &[u64; 16]) -> u64 {
    let shift = if case.high { 8 } else { 0 };
    let source = (u128::from(gpr[case.src]) >> shift) & mask(case.src_bits);
    let negative = case.signed && source >= 1u128 << (case.src_bits - 1);
    let extended = if negative {
        (1u128 << 64) - (1u128 << case.src_bits) + source
    } else {
        source
    };
    let low = extended & mask(case.dst_bits);
    let kept = if case.dst_bits == 16 {
        u128::from(gpr[case.dst]) & !mask(16)
    } else {
        0 // 32-bit writes clear bits 63:32.
    };
    (kept | low) as u64
}

fn memory_with(code: &[u8], data: &[u8]) -> GuestMemory {
    let mut memory = GuestMemory::new();
    memory
        .map_region_with_data(ENTRY, code.len(), MemoryPermissions::READ_EXECUTE, code)
        .unwrap();
    let mut bytes = vec![0xEEu8; DATA_LEN];
    bytes[..data.len()].copy_from_slice(data);
    memory
        .map_region_with_data(DATA, DATA_LEN, MemoryPermissions::READ_WRITE, &bytes)
        .unwrap();
    memory
}

fn interpret(cpu: &CpuState, memory: &mut GuestMemory) -> ExecutionResult {
    Interpreter::new(cpu.clone(), memory, 1).run()
}

/// Run one instruction as translated native code, proving that it was.
fn translate(cpu: &CpuState, memory: &mut GuestMemory, text: &str) -> ExecutionResult {
    let mut engine = DbtEngine::new(64 * 1024).expect("JIT memory");
    let result = engine.run(&mut cpu.clone(), memory, 1);
    if result.exit_reason == ExitReason::StepLimitExceeded {
        assert_eq!(engine.metrics.blocks_compiled, 1, "{text}");
        assert_eq!(engine.metrics.dbt_dispatches, 1, "{text}");
        assert_eq!(engine.metrics.instructions_executed, 1, "{text}");
        assert_eq!(engine.metrics.interpreter_fallbacks, 0, "{text}");
    }
    result
}

fn both(
    code: &[u8],
    data: &[u8],
    cpu: &CpuState,
    text: &str,
) -> [(ExecutionResult, GuestMemory); 2] {
    let mut first = memory_with(code, data);
    let mut second = memory_with(code, data);
    let interpreted = interpret(cpu, &mut first);
    let translated = translate(cpu, &mut second, text);
    [(interpreted, first), (translated, second)]
}

#[test]
fn register_extensions_match_the_architecture_in_both_engines() {
    let low_values: [u64; 12] = [
        0,
        1,
        0x7F,
        0x80,
        0xFF,
        0x100,
        0x7FFF,
        0x8000,
        0xFFFF,
        0x7FFF_FFFF,
        0x8000_0000,
        0xFFFF_FFFF,
    ];
    let patterns = low_values
        .into_iter()
        .flat_map(|low| [low, low | 0xA5A5_5A5A_0000_0000, low << 8]);
    for pattern in patterns {
        for case in CASES {
            for flags_set in [false, true] {
                let mut cpu = CpuState::new(ENTRY, 0);
                for (index, register) in cpu.gpr.iter_mut().enumerate() {
                    *register = 0x1122_3344_5566_7788u64.rotate_left(index as u32 * 4);
                }
                cpu.gpr[case.src] = pattern;
                cpu.flags = RFlags {
                    cf: flags_set,
                    zf: flags_set,
                    sf: flags_set,
                    of: flags_set,
                    ..RFlags::default()
                };
                let mut gpr = cpu.gpr;
                gpr[case.dst] = expected(case, &cpu.gpr);
                for (result, _) in both(case.code, &[], &cpu, case.text) {
                    assert_eq!(
                        result.exit_reason,
                        ExitReason::StepLimitExceeded,
                        "{}",
                        case.text
                    );
                    assert_eq!(result.instructions_executed, 1, "{}", case.text);
                    assert_eq!(result.final_state.gpr, gpr, "{} {pattern:#x}", case.text);
                    assert_eq!(result.final_state.flags, cpu.flags, "{}", case.text);
                    assert_eq!(result.final_state.rip, ENTRY + case.code.len() as u64);
                }
            }
        }
    }
}

#[test]
fn mov_immediate_is_never_decoded_as_a_sign_extension() {
    for &(code, register, value) in MOV_IMMEDIATE {
        let mut cpu = CpuState::new(ENTRY, 0);
        cpu.gpr = [0xFFFF_FFFF_FFFF_FFFF; 16];
        let mut gpr = cpu.gpr;
        gpr[register] = value;
        for (result, _) in both(code, &[], &cpu, "mov immediate") {
            assert_eq!(
                result.exit_reason,
                ExitReason::StepLimitExceeded,
                "{code:02X?}"
            );
            assert_eq!(result.final_state.gpr, gpr, "{code:02X?}");
            assert_eq!(result.final_state.rip, ENTRY + code.len() as u64);
        }
    }
}

#[test]
fn sixteen_bit_mov_immediate_writes_only_the_low_word() {
    // 66 B9 34 12 = mov cx, 0x1234
    let mut cpu = CpuState::new(ENTRY, 0);
    cpu.gpr = [0xFFFF_FFFF_FFFF_FFFF; 16];
    let mut gpr = cpu.gpr;
    gpr[Register::Rcx as usize] = 0xFFFF_FFFF_FFFF_1234;
    for (result, _) in both(&[0x66, 0xB9, 0x34, 0x12], &[], &cpu, "mov cx, 0x1234") {
        assert_eq!(result.exit_reason, ExitReason::StepLimitExceeded);
        assert_eq!(result.final_state.gpr, gpr);
        assert_eq!(result.final_state.rip, ENTRY + 4);
    }
}

#[test]
fn every_truncation_faults_without_changing_state() {
    let codes = CASES
        .iter()
        .map(|case| case.code)
        .chain(MOV_IMMEDIATE.iter().map(|entry| entry.0));
    for code in codes {
        for len in 1..code.len() {
            let mut cpu = CpuState::new(ENTRY, 0);
            cpu.gpr = [0x8080_8080_8080_8080; 16];
            for (result, _) in both(&code[..len], &[], &cpu, "truncated") {
                assert!(
                    matches!(result.exit_reason, ExitReason::MemoryFault(_)),
                    "{:02X?}: {:?}",
                    &code[..len],
                    result.exit_reason
                );
                assert_eq!(result.instructions_executed, 0);
                assert_eq!(result.final_state, cpu);
            }
        }
    }
}

#[test]
fn memory_extensions_use_checked_addresses_in_both_engines() {
    let data = [0x80u8, 0xFF, 0x7F, 0x00, 0xFE, 0x80, 0xFF, 0x81, 0xC2, 0x83];
    let cases: &[(&str, &[u8], u64)] = &[
        (
            "movsx eax, byte ptr [rbx]",
            &[0x0F, 0xBE, 0x03],
            0xFFFF_FF80,
        ),
        (
            "movsx rax, byte ptr [rbx+4]",
            &[0x48, 0x0F, 0xBE, 0x43, 0x04],
            0xFFFF_FFFF_FFFF_FFFE,
        ),
        (
            "movsx ax, byte ptr [rbx]",
            &[0x66, 0x0F, 0xBE, 0x03],
            0xAAAA_AAAA_AAAA_FF80,
        ),
        (
            "movsx rax, word ptr [rbx+rcx*2+4]",
            &[0x48, 0x0F, 0xBF, 0x44, 0x4B, 0x04],
            0xFFFF_FFFF_FFFF_81FF,
        ),
        (
            "movzx eax, byte ptr [rbx+4]",
            &[0x0F, 0xB6, 0x43, 0x04],
            0xFE,
        ),
        (
            "movsxd rax, dword ptr [rbx+4]",
            &[0x48, 0x63, 0x43, 0x04],
            0xFFFF_FFFF_81FF_80FE,
        ),
        (
            "movsxd rax, dword ptr [r8+rcx*4]",
            &[0x49, 0x63, 0x04, 0x88],
            0xFFFF_FFFF_81FF_80FE,
        ),
        // The next instruction starts at 0x1007, so the address is 0x2007.
        (
            "movsx eax, word ptr [rip+0x1000]",
            &[0x0F, 0xBF, 0x05, 0x00, 0x10, 0x00, 0x00],
            0xFFFF_C281,
        ),
        (
            "movsxd rax, dword ptr gs:[rbx]",
            &[0x65, 0x48, 0x63, 0x03],
            0x007F_FF80,
        ),
        (
            "movzx eax, byte ptr fs:[rbx]",
            &[0x64, 0x0F, 0xB6, 0x03],
            0xFF,
        ),
        ("mov eax, dword ptr [rbx]", &[0x8B, 0x03], 0x007F_FF80),
    ];
    for &(text, code, value) in cases {
        let mut cpu = CpuState::new(ENTRY, 0);
        cpu.gpr[Register::Rax as usize] = 0xAAAA_AAAA_AAAA_AAAA;
        cpu.gpr[Register::Rbx as usize] = DATA;
        cpu.gpr[Register::Rcx as usize] = 1;
        cpu.gpr[Register::R8 as usize] = DATA;
        match code[0] {
            0x65 => (cpu.gs_base, cpu.gpr[Register::Rbx as usize]) = (DATA, 0),
            0x64 => (cpu.fs_base, cpu.gpr[Register::Rbx as usize]) = (DATA - 4, 5),
            _ => {}
        }
        for (result, _) in both(code, &data, &cpu, text) {
            assert_eq!(result.exit_reason, ExitReason::StepLimitExceeded, "{text}");
            assert_eq!(result.final_state.gpr[0], value, "{text}");
            assert_eq!(result.final_state.rip, ENTRY + code.len() as u64, "{text}");
        }
    }
}

#[test]
fn memory_operands_crossing_the_region_end_fault_precisely() {
    // Only the bytes the operand really covers may be touched: the last word,
    // dword or byte of the region is readable, one byte further is not.
    let cases: &[(&[u8], u64)] = &[
        (&[0x0F, 0xBE, 0x03], 1),
        (&[0x0F, 0xBF, 0x03], 2),
        (&[0x48, 0x63, 0x03], 4),
        (&[0x8B, 0x03], 4), // mov eax, dword ptr [rbx]
        (&[0x89, 0x03], 4), // mov dword ptr [rbx], eax
    ];
    for &(code, width) in cases {
        let end = DATA + DATA_LEN as u64;
        for (address, ok) in [(end - width, true), (end - width + 1, false)] {
            let mut cpu = CpuState::new(ENTRY, 0);
            cpu.gpr[Register::Rbx as usize] = address;
            for (result, memory) in both(code, &[], &cpu, "boundary") {
                if ok {
                    assert_eq!(
                        result.exit_reason,
                        ExitReason::StepLimitExceeded,
                        "{code:02X?}"
                    );
                } else {
                    assert!(
                        matches!(result.exit_reason, ExitReason::MemoryFault(_)),
                        "{code:02X?}"
                    );
                    assert_eq!(result.instructions_executed, 0);
                    assert_eq!(result.final_state, cpu, "{code:02X?}");
                    let mut tail = [0u8; 4];
                    memory.read_bytes(end - 4, &mut tail).unwrap();
                    assert_eq!(tail, [0xEE; 4]);
                }
            }
        }
    }
}

#[test]
fn a_32_bit_store_leaves_the_next_four_bytes_unchanged() {
    // 89 03 = mov dword ptr [rbx], eax
    let mut cpu = CpuState::new(ENTRY, 0);
    cpu.gpr[Register::Rax as usize] = 0xDDDD_DDDD_1234_5678;
    cpu.gpr[Register::Rbx as usize] = DATA;
    for (result, memory) in both(&[0x89, 0x03], &[], &cpu, "mov dword ptr [rbx], eax") {
        assert_eq!(result.exit_reason, ExitReason::StepLimitExceeded);
        assert_eq!(memory.read_u64(DATA).unwrap(), 0xEEEE_EEEE_1234_5678);
    }
}

#[test]
fn native_sxtw_encodes_every_register() {
    // LLVM: sxtw x0, w30 then sxtw xN, w(31-N) for N in 1..=30.
    let llvm: [u32; 31] = [
        0x93407fc0, 0x93407fc1, 0x93407fa2, 0x93407f83, 0x93407f64, 0x93407f45, 0x93407f26,
        0x93407f07, 0x93407ee8, 0x93407ec9, 0x93407eaa, 0x93407e8b, 0x93407e6c, 0x93407e4d,
        0x93407e2e, 0x93407e0f, 0x93407df0, 0x93407dd1, 0x93407db2, 0x93407d93, 0x93407d74,
        0x93407d55, 0x93407d36, 0x93407d17, 0x93407cf8, 0x93407cd9, 0x93407cba, 0x93407c9b,
        0x93407c7c, 0x93407c5d, 0x93407c3e,
    ];
    let mut assembler = xenora_loader::Arm64Assembler::new();
    for xd in 0..31u8 {
        assembler.sxtw_64(xd, if xd == 0 { 30 } else { 31 - xd });
    }
    assert_eq!(assembler.instructions(), llvm);
}

#[test]
fn probe_reports_exact_extension_lengths_and_support() {
    use xenora_loader::probe::{ProbeConfig, probe_pe_bytes};
    for case in CASES {
        let report = probe_pe_bytes(&probe_image(case.code), &ProbeConfig::default()).unwrap();
        let record = &report.instruction_records[0];
        assert_eq!(record.bytes, case.code, "{}", case.text);
        assert!(
            record.is_supported_dbt && record.is_supported_interp,
            "{}",
            case.text
        );
    }
    for &(code, _, _) in MOV_IMMEDIATE {
        let report = probe_pe_bytes(&probe_image(code), &ProbeConfig::default()).unwrap();
        let record = &report.instruction_records[0];
        assert_eq!(record.bytes, code);
        assert!(
            record.mnemonic.starts_with("mov reg, imm"),
            "{}",
            record.mnemonic
        );
    }
}

/// Minimal PE32+ image whose entry point holds `code` followed by HLT.
fn probe_image(code: &[u8]) -> Vec<u8> {
    let mut pe = vec![0u8; 0x400];
    let mut put =
        |offset: usize, bytes: &[u8]| pe[offset..offset + bytes.len()].copy_from_slice(bytes);
    put(0, b"MZ");
    put(0x3C, &0x80u32.to_le_bytes());
    put(0x80, b"PE\0\0");
    put(0x84, &0x8664u16.to_le_bytes());
    put(0x86, &1u16.to_le_bytes());
    put(0x94, &240u16.to_le_bytes());
    put(0x98, &0x20Bu16.to_le_bytes());
    put(0x98 + 16, &(ENTRY as u32).to_le_bytes());
    put(0x98 + 24, &0x1_4000_0000u64.to_le_bytes());
    put(0x98 + 32, &0x1000u32.to_le_bytes());
    put(0x98 + 36, &0x200u32.to_le_bytes());
    put(0x98 + 56, &0x2000u32.to_le_bytes());
    put(0x98 + 60, &0x200u32.to_le_bytes());
    put(0x98 + 108, &16u32.to_le_bytes());
    let section = 0x98 + 240;
    put(section, b".text");
    put(section + 8, &0x200u32.to_le_bytes());
    put(section + 12, &(ENTRY as u32).to_le_bytes());
    put(section + 16, &0x200u32.to_le_bytes());
    put(section + 20, &0x200u32.to_le_bytes());
    put(section + 36, &0x6000_0020u32.to_le_bytes());
    put(0x200, code);
    put(0x200 + code.len(), &[0xF4]);
    pe
}
