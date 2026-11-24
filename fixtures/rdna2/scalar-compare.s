// SPDX-License-Identifier: GPL-3.0-or-later
// Original SOPC vectors: each of the twelve 32-bit integer compares sets SCC,
// and an s_cbranch_scc0 skips setting that compare's bit in s20, so s20
// records every result. -1 and 1 separate signed from unsigned ordering;
// inline and literal operands appear in the second position.
// Expected: input ^ 0x00000d5b (bits worked out by hand below).
.text
buffer_load_dword v1, v0, s[0:3], 0 idxen
s_waitcnt vmcnt(0)
s_mov_b32 s8, -1
s_mov_b32 s9, 1
s_mov_b32 s20, 0
s_cmp_eq_i32 s8, -1             // -1 == -1: SCC = 1
s_cbranch_scc0 .Lskip0
s_or_b32 s20, s20, 1
.Lskip0:
s_cmp_lg_i32 s8, s9             // -1 != 1: SCC = 1
s_cbranch_scc0 .Lskip1
s_or_b32 s20, s20, 2
.Lskip1:
s_cmp_gt_i32 s8, s9             // -1 > 1: SCC = 0
s_cbranch_scc0 .Lskip2
s_or_b32 s20, s20, 4
.Lskip2:
s_cmp_ge_i32 s9, s9             // 1 >= 1: SCC = 1
s_cbranch_scc0 .Lskip3
s_or_b32 s20, s20, 8
.Lskip3:
s_cmp_lt_i32 s8, s9             // -1 < 1: SCC = 1
s_cbranch_scc0 .Lskip4
s_or_b32 s20, s20, 16
.Lskip4:
s_cmp_le_i32 s9, s8             // 1 <= -1: SCC = 0
s_cbranch_scc0 .Lskip5
s_or_b32 s20, s20, 32
.Lskip5:
s_cmp_eq_u32 s9, 1              // 1 == 1: SCC = 1
s_cbranch_scc0 .Lskip6
s_or_b32 s20, s20, 64
.Lskip6:
s_cmp_lg_u32 s8, 0xffffffff     // 0xffffffff != 0xffffffff: SCC = 0
s_cbranch_scc0 .Lskip7
s_or_b32 s20, s20, 0x80
.Lskip7:
s_cmp_gt_u32 s8, s9             // 0xffffffff > 1: SCC = 1
s_cbranch_scc0 .Lskip8
s_or_b32 s20, s20, 0x100
.Lskip8:
s_cmp_ge_u32 s9, s8             // 1 >= 0xffffffff: SCC = 0
s_cbranch_scc0 .Lskip9
s_or_b32 s20, s20, 0x200
.Lskip9:
s_cmp_lt_u32 s9, s8             // 1 < 0xffffffff: SCC = 1
s_cbranch_scc0 .Lskip10
s_or_b32 s20, s20, 0x400
.Lskip10:
s_cmp_le_u32 s9, 64             // 1 <= 64: SCC = 1
s_cbranch_scc0 .Lskip11
s_or_b32 s20, s20, 0x800
.Lskip11:
v_mov_b32_e32 v2, s20
v_xor_b32_e32 v2, v1, v2            // s20 = 0x00000d5b
buffer_store_dword v2, v0, s[4:7], 0 idxen
s_waitcnt_vscnt null, 0
s_endpgm
