// SPDX-License-Identifier: GPL-3.0-or-later
.text
v_mov_b32_e32 v9, lit(0x3f000000)
v_mov_b32_e32 v10, 0x3e800000
v_mul_f32_e32 v8, v0, v9
v_mul_f32_e32 v9, v1, v9
v_add_f32_e32 v8, v8, v10
exp pos0, v8, v9, v2, v3 done
s_endpgm
