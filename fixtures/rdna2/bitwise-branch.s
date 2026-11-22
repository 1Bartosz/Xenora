// SPDX-License-Identifier: GPL-3.0-or-later
.text
buffer_load_dword v1, v0, s[0:3], 0 idxen
s_waitcnt vmcnt(0)
s_mov_b32 s8, 7
s_cmp_eq_u32 s8, 6
s_cbranch_scc0 .Lmask
v_mov_b32_e32 v2, 0
s_branch .Ldone
.Lmask:
v_mov_b32_e32 v2, 0x00ff00ff
v_and_b32_e32 v1, v1, v2
v_mov_b32_e32 v2, 0x12345678
v_xor_b32_e32 v2, v1, v2
s_branch .Ldone
.Ldone:
buffer_store_dword v2, v0, s[4:7], 0 idxen
s_waitcnt_vscnt null, 0
s_endpgm
