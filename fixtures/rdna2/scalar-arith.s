// SPDX-License-Identifier: GPL-3.0-or-later
// Original SOP2 vectors for the registry-declared arithmetic, select and
// bit-field forms: signed and unsigned min/max (including a tie, which gives S1
// and SCC = 0), select on both SCC values, bit-field mask, low and high
// multiplies, both bit-field extracts, absolute difference and the four
// shift-and-add forms. BFM, MUL, MUL_HI and CSELECT leave SCC unchanged; the
// last SCC writer (LSHL2_ADD, no carry) clears it for the s_cbranch_scc0.
// Values from the guide's 12.1 pseudocode, with s8 = 0x89abcdef (negative as
// i32) and s9 = 5.
// Expected: input ^ 0x81157dfe (the XOR of s10-s27 below).
.text
buffer_load_dword v1, v0, s[0:3], 0 idxen
s_waitcnt vmcnt(0)
s_mov_b32 s8, 0x89abcdef
s_mov_b32 s9, 5
s_min_i32 s10, s8, s9               // s10 = 0x89abcdef, SCC = 1 (S0 < S1 signed)
s_cselect_b32 s11, s9, s8           // s11 = 5 (SCC = 1 picks S0), SCC unchanged
s_max_u32 s12, s8, s9               // s12 = 0x89abcdef, SCC = 1
s_bfm_b32 s13, 12, 4                // s13 = 0xfff << 4 = 0xfff0, SCC unchanged
s_lshl1_add_u32 s14, s8, s9         // (s8 << 1) + 5 = 0x1_13579be3: s14 = 0x13579be3, SCC = 1
s_mul_i32 s15, s8, s9               // 5 * s8 = 0x2_b05b05ab: s15 = 0xb05b05ab
s_bfe_u32 s16, s8, 0x80004          // offset 4, width 8: s16 = 0xde, SCC = 1
s_bfe_i32 s17, s8, 0x80004          // 0xde sign-extended from bit 7: s17 = 0xffffffde, SCC = 1
s_absdiff_i32 s18, s9, s8           // 5 - (-0x76543211) = 0x76543216: s18, SCC = 1
s_lshl3_add_u32 s19, s8, s8         // 9 * s8 = 0x4_d70a3d67: s19 = 0xd70a3d67, SCC = 1
s_lshl4_add_u32 s20, s9, -16        // 80 + 0xfffffff0 = 0x1_00000040: s20 = 0x40, SCC = 1
s_mul_hi_u32 s21, s8, s8            // high word of s8 * s8 (unsigned): s21 = 0x4a0955b6
s_mul_hi_i32 s22, s8, s8            // high word of 0x76543211^2: s22 = 0x36b1b9d8
s_max_i32 s23, s8, s9               // s23 = 5, SCC = 0 (S0 not greater)
s_min_u32 s24, s9, 5                // tie: s24 = 5 (S1), SCC = 0
s_lshl2_add_u32 s25, s9, s9         // 20 + 5: s25 = 25, SCC = 0 (no carry)
s_cselect_b32 s26, s8, s9           // s26 = 5 (SCC = 0 picks S1), SCC unchanged
s_mul_hi_u32 s27, s9, s9            // s27 = 0, SCC unchanged (0)
s_cbranch_scc0 .Lzero
v_mov_b32_e32 v2, 0
s_branch .Ldone
.Lzero:
v_mov_b32_e32 v2, s10
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
v_mov_b32_e32 v3, s20
v_xor_b32_e32 v2, v2, v3
v_mov_b32_e32 v3, s21
v_xor_b32_e32 v2, v2, v3
v_mov_b32_e32 v3, s22
v_xor_b32_e32 v2, v2, v3
v_mov_b32_e32 v3, s23
v_xor_b32_e32 v2, v2, v3
v_mov_b32_e32 v3, s24
v_xor_b32_e32 v2, v2, v3
v_mov_b32_e32 v3, s25
v_xor_b32_e32 v2, v2, v3
v_mov_b32_e32 v3, s26
v_xor_b32_e32 v2, v2, v3
v_mov_b32_e32 v3, s27
v_xor_b32_e32 v2, v2, v3
v_xor_b32_e32 v2, v1, v2            // v2 = input ^ 0x81157dfe
.Ldone:
buffer_store_dword v2, v0, s[4:7], 0 idxen
s_waitcnt_vscnt null, 0
s_endpgm
