// SPDX-License-Identifier: GPL-3.0-or-later
// Original SOP2 vectors for the registry-declared logical and shift forms:
// ANDN2/ORN2 with a literal and an inline -1, NAND with aliased sources,
// NOR/XNOR chains, shift counts that use only S1[4:0] (36 -> 4, a literal
// 0xffffffe1 -> 1, 31), an arithmetic shift of a negative and a positive value,
// and an SCC branch on a zero shift result. SGPR results are moved into VGPRs.
// Expected: input ^ 0x887f00f8 (worked out by hand below).
.text
buffer_load_dword v1, v0, s[0:3], 0 idxen
s_waitcnt vmcnt(0)
s_mov_b32 s8, 0xf0f0ff00
s_andn2_b32 s9, s8, 0xff00ff00      // s9  = 0x00f00000, SCC = 1
s_orn2_b32 s10, s9, -1              // s10 = 0x00f00000, SCC = 1
s_nand_b32 s11, s8, s8              // s11 = 0x0f0f00ff, SCC = 1
s_nor_b32 s12, s11, s9              // s12 = 0xf000ff00, SCC = 1
s_xnor_b32 s13, s12, s8             // s13 = 0xff0fffff, SCC = 1
s_lshl_b32 s14, s13, 36             // s14 = 0xf0fffff0 (count 4), SCC = 1
s_lshr_b32 s15, s14, 0xffffffe1     // s15 = 0x787ffff8 (count 1), SCC = 1
s_ashr_i32 s16, s14, 31             // s16 = 0xffffffff, SCC = 1
s_ashr_i32 s17, s15, 31             // s17 = 0, SCC = 0
s_cbranch_scc0 .Lzero
v_mov_b32_e32 v2, 0
s_branch .Ldone
.Lzero:
v_mov_b32_e32 v2, s13
v_mov_b32_e32 v3, s15
v_xor_b32_e32 v2, v2, v3
v_mov_b32_e32 v4, s16
v_xor_b32_e32 v2, v2, v4
v_mov_b32_e32 v5, s10
v_xor_b32_e32 v2, v2, v5
v_mov_b32_e32 v6, s12
v_xor_b32_e32 v2, v2, v6            // 0xff0fffff ^ 0x787ffff8 ^ 0xffffffff ^ 0x00f00000 ^ 0xf000ff00
v_xor_b32_e32 v2, v1, v2
.Ldone:
buffer_store_dword v2, v0, s[4:7], 0 idxen
s_waitcnt_vscnt null, 0
s_endpgm
