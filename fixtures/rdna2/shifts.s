// SPDX-License-Identifier: GPL-3.0-or-later
.text
buffer_load_dword v1, v0, s[0:3], 0 idxen
s_waitcnt vmcnt(0)
v_mov_b32_e32 v255, 0x80000001
v_mov_b32_e32 v2, 0x80000003
v_lshrrev_b32_e32 v255, v1, v255
v_lshlrev_b32_e32 v1, v1, v2
v_xor_b32_e32 v1, v1, v255
buffer_store_dword v1, v0, s[4:7], 0 idxen
s_waitcnt_vscnt null, 0
s_endpgm
