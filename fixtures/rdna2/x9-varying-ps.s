// SPDX-License-Identifier: GPL-3.0-or-later
.text
v_interp_p1_f32 v8, v0, attr0.x
v_interp_p2_f32 v8, v1, attr0.x
v_interp_p1_f32 v9, v0, attr0.y
v_interp_p2_f32 v9, v1, attr0.y
v_interp_p1_f32 v10, v0, attr0.z
v_interp_p2_f32 v10, v1, attr0.z
v_interp_p1_f32 v11, v0, attr0.w
v_interp_p2_f32 v11, v1, attr0.w
exp mrt0, v8, v9, v10, v11 done vm
s_endpgm
