// SPDX-License-Identifier: GPL-3.0-or-later
.text
buffer_load_dword v1, v0, s[0:3], 0 idxen
s_waitcnt vmcnt(0)
v_mov_b32_e32 v255, 0x805500aa
v_or_b32_e32 v1, v1, v255
buffer_store_dword v1, v0, s[4:7], 0 idxen
s_waitcnt_vscnt null, 0
s_endpgm
