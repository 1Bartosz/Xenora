// SPDX-License-Identifier: GPL-3.0-or-later
// Original SOP1 vectors for the registry-declared unary forms: complement, bit
// reverse, both bit counts, both find-first searches (one finding nothing, which
// gives -1), both find-last-bit forms, sign extension from 8 and 16 bits, and
// absolute value. SCC is set by NOT/BCNT/ABS only: an ABS of zero clears it, and
// the following SEXT and FF1 leave it clear for the s_cbranch_scc0.
// Expected: input ^ 0x0f0080f5 (worked out by hand below).
.text
buffer_load_dword v1, v0, s[0:3], 0 idxen
s_waitcnt vmcnt(0)
s_mov_b32 s8, 0xf0
s_not_b32 s9, s8                    // s9  = 0xffffff0f, SCC = 1
s_brev_b32 s10, s8                  // s10 = 0x0f000000
s_bcnt1_i32_b32 s11, s8             // s11 = 4, SCC = 1
s_bcnt0_i32_b32 s12, s8             // s12 = 28, SCC = 1
s_ff1_i32_b32 s13, 0x400000         // s13 = 22
s_ff0_i32_b32 s14, 0xffff           // s14 = 16
s_flbit_i32_b32 s15, s8             // s15 = 24
s_flbit_i32 s16, 0xfff00000         // s16 = 12 (bits equal to the sign above bit 19)
s_sext_i32_i8 s17, s8               // s17 = 0xfffffff0
s_sext_i32_i16 s18, 0x18000         // s18 = 0xffff8000
s_abs_i32 s19, s18                  // s19 = 0x8000, SCC = 1
s_abs_i32 s20, 0                    // s20 = 0, SCC = 0
s_sext_i32_i16 s21, 0x7fff          // s21 = 0x7fff, SCC unchanged (0)
s_ff1_i32_b32 s22, 0                // s22 = 0xffffffff (none found), SCC unchanged (0)
s_cbranch_scc0 .Lzero
v_mov_b32_e32 v2, 0
s_branch .Ldone
.Lzero:
v_mov_b32_e32 v2, s9
v_mov_b32_e32 v3, s10
v_xor_b32_e32 v2, v2, v3
v_mov_b32_e32 v3, s11
v_xor_b32_e32 v2, v2, v3
v_mov_b32_e32 v3, s12
v_xor_b32_e32 v2, v2, v3
v_mov_b32_e32 v3, s13
v_xor_b32_e32 v2, v2, v3
v_mov_b32_e32 v3, s14
v_xor_b32_e32 v2, v2, v3
v_mov_b32_e32 v3, s15
v_xor_b32_e32 v2, v2, v3
v_mov_b32_e32 v3, s16
v_xor_b32_e32 v2, v2, v3
v_mov_b32_e32 v3, s17
v_xor_b32_e32 v2, v2, v3
v_mov_b32_e32 v3, s18
v_xor_b32_e32 v2, v2, v3
v_mov_b32_e32 v3, s19
v_xor_b32_e32 v2, v2, v3
v_mov_b32_e32 v3, s21
v_xor_b32_e32 v2, v2, v3
v_mov_b32_e32 v3, s22
v_xor_b32_e32 v2, v2, v3
v_xor_b32_e32 v2, v1, v2            // v2 = input ^ 0x0f0080f5
.Ldone:
buffer_store_dword v2, v0, s[4:7], 0 idxen
s_waitcnt_vscnt null, 0
s_endpgm
