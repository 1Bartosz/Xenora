# SPDX-License-Identifier: GPL-3.0-or-later
.text
buffer_load_dword v1, v0, s[0:3], 0 idxen
s_waitcnt vmcnt(0)
v_mov_b32_e32 v2, 0x01020304
v_xor_b32_e32 v1, v1, v2
buffer_store_dword v1, v0, s[4:7], 0 idxen
s_waitcnt_vscnt null, 0
s_endpgm
