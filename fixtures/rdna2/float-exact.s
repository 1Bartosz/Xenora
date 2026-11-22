// SPDX-License-Identifier: GPL-3.0-or-later
.text
buffer_load_dword v1, v0, s[0:3], 0 idxen
s_waitcnt vmcnt(0)
v_mov_b32_e32 v3, 0x00400000
v_and_b32_e32 v1, v1, v3
v_mov_b32_e32 v3, 0x3f800000
v_xor_b32_e32 v1, v1, v3
v_mov_b32_e32 v3, 0x40200000
v_add_f32_e32 v1, v1, v3
v_mul_f32_e32 v1, v1, v3
buffer_store_dword v1, v0, s[4:7], 0 idxen
s_waitcnt_vscnt null, 0
s_endpgm
