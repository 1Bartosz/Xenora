// SPDX-License-Identifier: GPL-3.0-or-later
// Original Wave32 VOPC vectors: all twelve 32-bit integer compares with VGPR,
// SGPR, inline, literal and vcc_lo sources; a compare under partial EXEC and
// a mask move into a seeded VGPR under partial EXEC; a compare under empty
// EXEC that must clear a stale VCC; SCC kept across compares; and a runtime
// s_cbranch_vccz. Each wave folds its masks into v2 as v2 = (v2 << 1) ^ mask.
// Expected results come from exact integer definitions in
// tests/validation/known_answers.rs, not from this file.
.text
buffer_load_dword v1, v0, s[0:3], 0 idxen
s_waitcnt vmcnt(0)
s_cmp_eq_u32 0, 0                     // SCC = 1, must survive every compare
s_mov_b32 s8, 0x7fffffff
v_mov_b32_e32 v2, 0                   // accumulator
v_mov_b32_e32 v5, 1                   // shift count
v_mov_b32_e32 v6, 0x0f0f0f0f
v_mov_b32_e32 v7, 0xa5a5a5a5          // seeded destination for a partial move
v_mov_b32_e32 v3, -1
v_cmp_lt_i32_e32 vcc_lo, v1, v3       // signed x < -1
v_mov_b32_e32 v4, vcc_lo
v_lshlrev_b32_e32 v2, v5, v2
v_xor_b32_e32 v2, v2, v4
v_cmp_lt_u32_e32 vcc_lo, v1, v3       // unsigned x < 0xffffffff
v_mov_b32_e32 v4, vcc_lo
v_lshlrev_b32_e32 v2, v5, v2
v_xor_b32_e32 v2, v2, v4
v_cmp_eq_i32_e32 vcc_lo, 0, v1
v_mov_b32_e32 v4, vcc_lo
v_lshlrev_b32_e32 v2, v5, v2
v_xor_b32_e32 v2, v2, v4
v_cmp_eq_u32_e32 vcc_lo, 0x80000000, v1
v_mov_b32_e32 v4, vcc_lo
v_lshlrev_b32_e32 v2, v5, v2
v_xor_b32_e32 v2, v2, v4
v_cmp_le_i32_e32 vcc_lo, s8, v1       // 0x7fffffff <= x (signed)
v_mov_b32_e32 v4, vcc_lo
v_lshlrev_b32_e32 v2, v5, v2
v_xor_b32_e32 v2, v2, v4
v_cmp_le_u32_e32 vcc_lo, s8, v1       // 0x7fffffff <= x (unsigned)
v_mov_b32_e32 v4, vcc_lo
v_lshlrev_b32_e32 v2, v5, v2
v_xor_b32_e32 v2, v2, v4
v_cmp_gt_i32_e32 vcc_lo, -16, v1      // -16 > x (signed)
v_mov_b32_e32 v4, vcc_lo
v_lshlrev_b32_e32 v2, v5, v2
v_xor_b32_e32 v2, v2, v4
v_cmp_gt_u32_e32 vcc_lo, 64, v1       // 64 > x (unsigned)
v_mov_b32_e32 v4, vcc_lo
v_lshlrev_b32_e32 v2, v5, v2
v_xor_b32_e32 v2, v2, v4
v_cmp_ne_i32_e32 vcc_lo, v1, v1       // aliased sources: never true
v_cmp_ne_u32_e32 vcc_lo, vcc_lo, v1   // reads the previous (zero) mask
v_mov_b32_e32 v4, vcc_lo
v_lshlrev_b32_e32 v2, v5, v2
v_xor_b32_e32 v2, v2, v4
v_cmp_ge_i32_e32 vcc_lo, 0x80000001, v1
v_mov_b32_e32 v4, vcc_lo
v_lshlrev_b32_e32 v2, v5, v2
v_xor_b32_e32 v2, v2, v4
s_mov_b32 exec_lo, 0x55555555
v_cmp_ge_u32_e32 vcc_lo, v1, v6       // only even lanes take part
v_mov_b32_e32 v7, vcc_lo              // odd lanes keep 0xa5a5a5a5
s_mov_b32 exec_lo, -1
v_xor_b32_e32 v2, v2, v7
s_mov_b32 vcc_lo, -1                  // stale nonzero mask
s_mov_b32 exec_lo, 0
v_cmp_eq_u32_e32 vcc_lo, v1, v1       // empty EXEC: VCC_LO = 0
s_cbranch_execz .Lempty
s_mov_b32 exec_lo, -1                 // reached only if EXECZ were ignored
v_mov_b32_e32 v2, 0
.Lempty:
s_mov_b32 exec_lo, -1
v_mov_b32_e32 v4, vcc_lo
v_xor_b32_e32 v2, v2, v4
s_cbranch_scc0 .Ldone                 // never taken: SCC is still 1
v_cmp_gt_u32_e32 vcc_lo, v1, v6       // runtime branch on lane data
s_cbranch_vccz .Ldone
v_xor_b32_e32 v2, v2, v6
.Ldone:
buffer_store_dword v2, v0, s[4:7], 0 idxen
s_waitcnt_vscnt null, 0
s_endpgm
