// SPDX-License-Identifier: GPL-3.0-or-later
.text
v_mov_b32_e32 v8, 1.0
v_mov_b32_e32 v9, 0
exp mrt0, v8, v9, v9, v8 done vm
s_endpgm
