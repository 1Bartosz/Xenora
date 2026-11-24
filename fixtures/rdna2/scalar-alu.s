// SPDX-License-Identifier: GPL-3.0-or-later
// Original SOP2 vectors: a carry chain through s_add_u32/s_addc_u32, a borrow
// chain through s_sub_u32/s_subb_u32, bitwise results feeding SCC, an SCC
// branch, and SGPR results moved into VGPRs. Expected: input ^ 0x8000000e.
.text
buffer_load_dword v1, v0, s[0:3], 0 idxen
s_waitcnt vmcnt(0)
s_mov_b32 s8, 0xfffffffe
s_add_u32 s8, s8, 3                 // s8 = 1, SCC = 1 (carry-out; dst aliases src0)
s_addc_u32 s9, 0x7fffffff, 0        // s9 = 0x80000000, SCC = 0
s_sub_u32 s10, 0, 1                 // s10 = 0xffffffff, SCC = 1 (borrow)
s_subb_u32 s11, s9, s8              // s11 = 0x7ffffffe, SCC = 0
s_xor_b32 s12, s11, s10             // s12 = 0x80000001, SCC = 1
s_and_b32 s13, s12, s9              // s13 = 0x80000000, SCC = 1
s_or_b32 s14, s13, -16              // s14 = 0xfffffff0, SCC = 1
s_and_b32 s15, s14, 15              // s15 = 0, SCC = 0
s_cbranch_scc0 .Lzero
v_mov_b32_e32 v2, 0
s_branch .Ldone
.Lzero:
v_mov_b32_e32 v2, s11
v_mov_b32_e32 v3, s14
v_xor_b32_e32 v2, v2, v3
v_xor_b32_e32 v2, v1, v2
.Ldone:
buffer_store_dword v2, v0, s[4:7], 0 idxen
s_waitcnt_vscnt null, 0
s_endpgm
