// SPDX-License-Identifier: GPL-3.0-or-later
// Original VOP2 vectors for the registry-declared per-lane integer forms: the
// four 24-bit multiplies, signed and unsigned min/max, XNOR and the three
// no-carry add/subtract forms (SUBREV reads its sources reversed). Every result
// depends on the lane's own input word x = v1, with SRC0 taken from an inline
// constant, a literal, an SGPR or a VGPR. No form writes SCC or VCC.
// Expected per lane: v14 = v2 ^ v3 ^ ... ^ v13, worked out from the guide's
// 12.7 pseudocode in tests/validation/known_answers.rs (vector_integer_key).
.text
buffer_load_dword v1, v0, s[0:3], 0 idxen
s_waitcnt vmcnt(0)
s_mov_b32 s8, 0x800001              // as i24: -0x7fffff
v_mul_i32_i24_e32 v2, -3, v1        // v2  = sext24(x) * -3 (low 32 bits)
v_mul_hi_i32_i24_e32 v3, s8, v1     // v3  = (-0x7fffff * sext24(x)) >> 32
v_mul_u32_u24_e32 v4, 0x123456, v1  // v4  = 0x123456 * x[23:0] (low 32 bits)
v_mul_hi_u32_u24_e32 v5, s8, v1     // v5  = (0x800001 * x[23:0]) >> 32
v_min_i32_e32 v6, v1, v2            // v6  = signed min(x, v2)
v_max_i32_e32 v7, 5, v1             // v7  = signed max(5, x)
v_min_u32_e32 v8, v3, v1            // v8  = unsigned min(v3, x)
v_max_u32_e32 v9, s8, v1            // v9  = unsigned max(0x800001, x)
v_xnor_b32_e32 v10, v4, v5          // v10 = ~(v4 ^ v5)
v_add_nc_u32_e32 v11, v6, v7        // v11 = v6 + v7
v_sub_nc_u32_e32 v12, v8, v9        // v12 = v8 - v9
v_subrev_nc_u32_e32 v13, v10, v11   // v13 = v11 - v10
v_xor_b32_e32 v14, v2, v3
v_xor_b32_e32 v14, v4, v14
v_xor_b32_e32 v14, v5, v14
v_xor_b32_e32 v14, v6, v14
v_xor_b32_e32 v14, v7, v14
v_xor_b32_e32 v14, v8, v14
v_xor_b32_e32 v14, v9, v14
v_xor_b32_e32 v14, v10, v14
v_xor_b32_e32 v14, v11, v14
v_xor_b32_e32 v14, v12, v14
v_xor_b32_e32 v14, v13, v14
buffer_store_dword v14, v0, s[4:7], 0 idxen
s_waitcnt_vscnt null, 0
s_endpgm
