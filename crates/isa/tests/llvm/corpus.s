// SPDX-License-Identifier: GPL-3.0-or-later
// Original gfx1030 Wave32 instruction identification cases.
s_mov_b32 s105, s0 // SOP1
s_mov_b32 exec_lo, -16 // SOP1
s_mov_b32 m0, 64 // SOP1
s_mov_b32 vcc_hi, 0xdeadbeef // SOP1
s_mov_b64 exec, vcc // SOP1
s_mov_b64 vcc, 0x12345678 // SOP1
s_not_b32 s0, s105 // SOP1
s_not_b64 s[104:105], s[0:1] // SOP1
s_brev_b32 s105, m0 // SOP1
s_bcnt1_i32_b32 s0, exec_lo // SOP1
s_ff1_i32_b32 s105, -16 // SOP1
s_flbit_i32_b32 s105, 0x80000100 // SOP1
s_sext_i32_i8 s0, s105 // SOP1
s_sext_i32_i16 s105, s0 // SOP1
s_getpc_b64 s[104:105] // SOP1
s_setpc_b64 s[0:1] // SOP1
s_and_saveexec_b64 s[0:1], s[104:105] // SOP1
s_wqm_b64 s[104:105], exec // SOP1
s_add_u32 s105, s0, 0x12345678 // SOP2
s_add_u32 s0, 0xdeadbeef, s105 // SOP2
s_add_i32 s0, -16, 64 // SOP2
s_sub_i32 s105, s0, m0 // SOP2
s_subb_u32 s0, vcc_lo, exec_lo // SOP2
s_min_i32 s105, s104, -16 // SOP2
s_max_u32 s0, s105, 64 // SOP2
s_cselect_b32 s105, -1, s0 // SOP2
s_and_b64 s[104:105], exec, vcc // SOP2
s_or_b64 vcc, s[0:1], exec // SOP2
s_xor_b32 s0, s0, 0xdeadbeef // SOP2
s_nand_b32 s105, m0, vcc_lo // SOP2
s_lshl_b32 s0, s105, 31 // SOP2
s_lshr_b64 s[104:105], s[0:1], 63 // SOP2
s_ashr_i32 s105, 0x87654321, 31 // SOP2
s_mul_i32 s0, s105, -16 // SOP2
s_bfe_u32 s105, s0, 0x0010000f // SOP2
s_cmp_eq_i32 s0, s105 // SOPC
s_cmp_lg_i32 m0, -16 // SOPC
s_cmp_gt_i32 64, s105 // SOPC
s_cmp_ge_i32 s105, 0xdeadbeef // SOPC
s_cmp_lt_u32 0x12345678, exec_hi // SOPC
s_cmp_le_u32 vcc_lo, 64 // SOPC
s_cmp_eq_u64 s[104:105], vcc // SOPC
s_bitcmp0_b32 s105, 31 // SOPC
s_bitcmp1_b64 exec, 63 // SOPC
s_movk_i32 s0, -32768 // SOPK
s_movk_i32 s105, 32767 // SOPK
s_movk_i32 m0, -1 // SOPK
s_addk_i32 s105, -32768 // SOPK
s_mulk_i32 s0, 32767 // SOPK
s_cmpk_eq_i32 s105, -32768 // SOPK
s_cmpk_lg_u32 s0, 65535 // SOPK
s_getreg_b32 s105, hwreg(HW_REG_MODE) // SOPK
s_setreg_b32 hwreg(HW_REG_MODE, 4, 8), s0 // SOPK
s_setreg_imm32_b32 hwreg(HW_REG_MODE, 8, 16), 0xdeadbeef // SOPK
s_setreg_imm32_b32 hwreg(HW_REG_MODE), 0 // SOPK
s_nop 15 // SOPP
s_endpgm 0 // SOPP
s_branch -32768 // SOPP
s_cbranch_scc0 32767 // SOPP
s_cbranch_scc1 -1 // SOPP
s_cbranch_vccz 0 // SOPP
s_cbranch_vccnz 3 // SOPP
s_cbranch_execz 7 // SOPP
s_cbranch_execnz -7 // SOPP
s_barrier // SOPP
s_waitcnt vmcnt(0) lgkmcnt(1) expcnt(7) // SOPP
s_sleep 127 // SOPP
s_trap 255 // SOPP
s_load_dword s105, s[0:1], 1048575 // SMEM
s_load_dword s0, s[104:105], -1048576 // SMEM
s_load_dwordx2 s[104:105], s[0:1], 0 glc // SMEM
s_load_dwordx4 s[0:3], s[100:101], s105 // SMEM
s_load_dwordx8 s[96:103], s[0:1], 7 // SMEM
s_load_dwordx16 s[0:15], s[104:105], 16 // SMEM
s_buffer_load_dword s105, s[0:3], 4095 // SMEM
s_buffer_load_dwordx2 s[104:105], s[0:3], 0 // SMEM
s_buffer_load_dwordx4 s[0:3], s[100:103], s105 // SMEM
s_memtime s[104:105] // SMEM
s_dcache_inv // SMEM
v_mov_b32_e32 v255, v0 // VOP1
v_mov_b32_e32 v0, s105 // VOP1
v_mov_b32_e32 v255, 0xdeadbeef // VOP1
v_mov_b32_e32 v0, -16 // VOP1
v_mov_b32_e32 v255, 64 // VOP1
v_mov_b32_e32 v0, 0.5 // VOP1
v_mov_b32_e32 v255, -4.0 // VOP1
v_mov_b32_e32 v0, 0.15915494 // VOP1
v_cvt_f32_i32_e32 v255, v0 // VOP1
v_cvt_f32_u32_e32 v0, v255 // VOP1
v_cvt_i32_f32_e32 v255, 0x3f800001 // VOP1
v_rcp_f32_e32 v0, v255 // VOP1
v_sqrt_f32_e32 v255, v0 // VOP1
v_readfirstlane_b32 s105, v255 // VOP1
v_mov_b32_sdwa v255, v0 dst_sel:BYTE_3 dst_unused:UNUSED_PRESERVE src0_sel:WORD_0 // VOP1
v_mov_b32_dpp v0, v255 quad_perm:[3,2,1,0] row_mask:0x1 bank_mask:0x8 // VOP1
v_mov_b32_dpp v255, v0 row_shl:15 bound_ctrl:1 // VOP1
v_mov_b32_dpp v0, v255 row_share:15 // VOP1
v_mov_b32_dpp v255, v0 dpp8:[7,6,5,4,3,2,1,0] fi:0 // VOP1
v_mov_b32_dpp v0, v255 dpp8:[0,0,0,0,7,7,7,7] fi:1 // VOP1
v_mov_b32_e32 v255, 0x3e22f983 // VOP1
v_add_f32_e32 v0, v255, v0 // VOP2
v_sub_f32_e32 v255, 0x3f800001, v0 // VOP2
v_subrev_f32_e32 v0, s105, v255 // VOP2
v_mul_f32_e32 v255, -4.0, v0 // VOP2
v_mul_i32_i24_e32 v0, -16, v255 // VOP2
v_mul_u32_u24_e32 v255, 64, v0 // VOP2
v_min_f32_e32 v0, 0.5, v255 // VOP2
v_max_f32_e32 v255, 0.15915494, v0 // VOP2
v_lshrrev_b32_e32 v0, 31, v255 // VOP2
v_ashrrev_i32_e32 v255, 31, v0 // VOP2
v_lshlrev_b32_e32 v0, 31, v255 // VOP2
v_and_b32_e32 v255, 0xdeadbeef, v0 // VOP2
v_or_b32_e32 v0, m0, v255 // VOP2
v_xor_b32_e32 v255, vcc_lo, v0 // VOP2
v_add_nc_u32_e32 v0, exec_lo, v255 // VOP2
v_fmamk_f32 v255, v0, 0x3f800001, v255 // VOP2
v_fmaak_f32 v0, v255, v0, 0xdeadbeef // VOP2
v_add_f32_sdwa v255, v0, v255 dst_sel:WORD_1 dst_unused:UNUSED_SEXT src0_sel:BYTE_0 src1_sel:WORD_0 // VOP2
v_add_f32_dpp v0, v255, v0 row_shl:7 bound_ctrl:1 // VOP2
v_mul_f32_dpp v255, v0, v255 dpp8:[1,2,3,4,5,6,7,0] fi:1 // VOP2
v_cmp_eq_f32_e32 vcc_lo, 0.5, v255 // VOPC
v_cmp_lt_f32_e32 vcc_lo, -4.0, v0 // VOPC
v_cmp_ge_f32_e32 vcc_lo, 0x3f800001, v255 // VOPC
v_cmp_ne_u32_e32 vcc_lo, 0xdeadbeef, v0 // VOPC
v_cmp_gt_i32_e32 vcc_lo, -16, v255 // VOPC
v_cmp_le_i32_e32 vcc_lo, s105, v0 // VOPC
v_cmpx_eq_u32_e32 64, v255 // VOPC
v_cmp_eq_i32_sdwa s105, v255, v0 src0_sel:WORD_0 src1_sel:BYTE_3 // VOPC
v_cmp_lt_u32_sdwa vcc_lo, v0, v255 src0_sel:BYTE_1 src1_sel:WORD_1 // VOPC
v_cmp_class_f32_e32 vcc_lo, v0, v255 // VOPC
v_add_f32_e64 v255, -v0, |v255| clamp // VOP3
v_add_f32_e64 v0, 0x3f800001, v255 // VOP3
v_mul_f32_e64 v255, v0, 0.5 mul:2 // VOP3
v_fma_f32 v0, v255, 0x3f800001, v0 // VOP3
v_fma_f32 v255, -v0, |v255|, -4.0 // VOP3
v_mad_u32_u24 v0, v255, 64, v0 // VOP3
v_mad_i32_i24 v255, v0, -16, v255 // VOP3
v_min3_f32 v0, v255, v0, 0.5 // VOP3
v_max3_i32 v255, v0, v255, -16 // VOP3
v_med3_u32 v0, v255, v0, 64 // VOP3
v_bfe_u32 v255, v0, 31, 1 // VOP3
v_bfe_i32 v0, v255, 0, 32 // VOP3
v_bfi_b32 v255, v0, v255, 0x12345678 // VOP3
v_alignbit_b32 v0, v255, v0, 31 // VOP3
v_cmp_eq_u32_e64 s105, v255, 0xdeadbeef // VOP3
v_readlane_b32 s105, v255, 31 // VOP3
v_writelane_b32 v255, s105, 31 // VOP3
v_add_co_u32 v255, s105, v0, v255 // VOP3SDST
v_add_co_u32 v0, vcc_lo, 0xdeadbeef, v255 // VOP3SDST
v_sub_co_u32 v255, s0, v0, v255 // VOP3SDST
v_subrev_co_u32 v0, s105, v255, v0 // VOP3SDST
v_div_scale_f32 v255, s105, v0, v0, v255 // VOP3SDST
v_div_scale_f64 v[254:255], vcc_lo, v[0:1], v[0:1], v[2:3] // VOP3SDST
v_pk_add_f16 v255, v0, v255 op_sel:[1,0] op_sel_hi:[0,1] // VOP3P
v_pk_mul_f16 v0, v255, 0x3c01 // VOP3P
v_pk_fma_f16 v255, v0, v255, 0x3c01 // VOP3P
v_pk_add_i16 v0, v255, 64 clamp // VOP3P
v_pk_sub_i16 v255, v0, -16 // VOP3P
v_pk_mul_lo_u16 v0, v255, v0 // VOP3P
v_pk_min_i16 v255, v0, v255 // VOP3P
v_pk_max_u16 v0, v255, v0 // VOP3P
v_pk_lshlrev_b16 v255, 15, v0 // VOP3P
v_dot2_f32_f16 v0, v255, v0, 0.5 // VOP3P
v_interp_p1_f32 v255, v0, attr32.w // VINTRP
v_interp_p2_f32 v0, v255, attr0.x // VINTRP
v_interp_mov_f32 v255, p0, attr32.y // VINTRP
v_interp_mov_f32 v0, p10, attr1.z // VINTRP
v_interp_p1_f32 v0, v255, attr1.y // VINTRP
v_interp_p2_f32 v255, v0, attr31.z // VINTRP
ds_write_b32 v255, v0 offset:65535 // DS
ds_read_b32 v0, v255 offset:65535 // DS
ds_write_b8 v0, v255 offset:0 // DS
ds_write_b16 v255, v0 offset:32768 // DS
ds_read_u8 v0, v255 offset:1 // DS
ds_read_i16 v255, v0 offset:65534 // DS
ds_write_b64 v255, v[254:255] offset:8 // DS
ds_read_b64 v[0:1], v255 offset:16 // DS
ds_write2_b32 v0, v255, v0 offset0:255 offset1:0 // DS
ds_read2_b32 v[254:255], v0 offset0:0 offset1:255 // DS
ds_add_u32 v255, v0 offset:65535 // DS
ds_add_rtn_u32 v0, v255, v0 offset:1 // DS
ds_cmpst_b32 v0, v255, v0 offset:32 // DS
ds_bpermute_b32 v255, v0, v255 // DS
ds_permute_b32 v0, v255, v0 // DS
buffer_load_dword v255, off, s[100:103], 0 offset:4095 // MUBUF
buffer_load_dword v0, v255, s[0:3], s105 offen offset:1 // MUBUF
buffer_load_dword v255, v0, s[0:3], 0 idxen // MUBUF
buffer_load_dword v0, v[254:255], s[0:3], 0 idxen offen glc slc // MUBUF
buffer_load_ubyte v255, off, s[0:3], 0 // MUBUF
buffer_load_sshort v0, off, s[0:3], 0 // MUBUF
buffer_load_dwordx2 v[254:255], off, s[0:3], 0 offset:4095 // MUBUF
buffer_load_dwordx3 v[0:2], off, s[0:3], 0 // MUBUF
buffer_store_dwordx4 v[252:255], off, s[100:103], 0 offset:4095 // MUBUF
buffer_store_byte v0, v255, s[0:3], s105 offen // MUBUF
buffer_atomic_add v255, off, s[0:3], 0 glc // MUBUF
buffer_atomic_cmpswap v[254:255], off, s[0:3], 0 glc // MUBUF
tbuffer_load_format_x v255, off, s[100:103], 0 format:[BUF_FMT_32_UINT] offset:4095 // MTBUF
tbuffer_load_format_xy v[0:1], v255, s[0:3], s105 format:[BUF_FMT_32_32_FLOAT] offen // MTBUF
tbuffer_load_format_xyz v[253:255], off, s[0:3], 0 format:[BUF_FMT_32_32_32_FLOAT] // MTBUF
tbuffer_load_format_xyzw v[0:3], off, s[0:3], 0 format:[BUF_FMT_32_32_32_32_FLOAT] // MTBUF
tbuffer_store_format_x v255, v0, s[100:103], 0 format:[BUF_FMT_32_SINT] idxen // MTBUF
tbuffer_store_format_xy v[254:255], off, s[0:3], 0 format:[BUF_FMT_32_32_UINT] // MTBUF
tbuffer_store_format_xyz v[0:2], off, s[0:3], 0 format:[BUF_FMT_32_32_32_UINT] // MTBUF
tbuffer_store_format_xyzw v[252:255], off, s[0:3], 0 format:[BUF_FMT_32_32_32_32_UINT] offset:4095 // MTBUF
tbuffer_load_format_d16_xy v255, off, s[0:3], 0 format:[BUF_FMT_16_16_FLOAT] // MTBUF
tbuffer_store_format_d16_xyzw v[254:255], off, s[0:3], 0 format:[BUF_FMT_16_16_16_16_FLOAT] // MTBUF
image_load v255, v0, s[96:103] dmask:0x1 dim:SQ_RSRC_IMG_1D unorm // MIMG
image_load_mip v[254:255], v[0:2], s[0:7] dmask:0x9 dim:SQ_RSRC_IMG_2D unorm // MIMG
image_store v255, v[0:1], s[0:7] dmask:0x8 dim:SQ_RSRC_IMG_2D unorm // MIMG
image_sample v[252:255], [v255,v0], s[0:7], s[100:103] dmask:0xf dim:SQ_RSRC_IMG_2D // MIMG
image_sample v255, [v0,v128,v255], s[0:7], s[8:11] dmask:0x1 dim:SQ_RSRC_IMG_3D // MIMG
image_sample_c v[254:255], [v255,v5,v1,v0], s[0:7], s[8:11] dmask:0x5 dim:SQ_RSRC_IMG_2D_ARRAY // MIMG
image_sample_c_l v255, [v0,v2,v4,v6,v255], s[0:7], s[8:11] dmask:0x1 dim:SQ_RSRC_IMG_2D_ARRAY // MIMG
image_sample_d v255, [v0,v2,v4,v6,v8,v255], s[0:7], s[8:11] dmask:0x1 dim:SQ_RSRC_IMG_2D // MIMG
image_sample_d v0, [v255,v2,v4,v6,v8,v10,v12,v14,v16], s[0:7], s[8:11] dmask:0x1 dim:SQ_RSRC_IMG_3D // MIMG
image_gather4 v[252:255], v[0:1], s[0:7], s[8:11] dmask:0x8 dim:SQ_RSRC_IMG_2D // MIMG
image_atomic_add v255, v[0:1], s[0:7] dmask:0x1 dim:SQ_RSRC_IMG_2D unorm glc // MIMG
image_sample_d_cl v255, [v0,v2,v4,v6,v8,v10,v12,v14,v16,v18], s[0:7], s[8:11] dmask:0x1 dim:SQ_RSRC_IMG_3D // MIMG
image_sample_c_d_cl v0, [v0,v2,v4,v6,v8,v10,v12,v14,v16,v18,v20], s[0:7], s[8:11] dmask:0x1 dim:SQ_RSRC_IMG_3D // MIMG
exp mrt0 v255, off, off, v0 done // EXP
exp mrt7 off, v255, v0, off // EXP
exp pos0 v0, v255, v0, v255 done // EXP
exp pos3 v255, v0, v255, v0 // EXP
exp param0 v255, off, v0, off // EXP
exp param31 v0, v255, off, off // EXP
exp null off, off, off, off done // EXP
flat_load_dword v255, v[0:1] offset:2047 // FLAT
flat_load_dword v0, v[254:255] offset:1024 // FLAT
flat_load_ubyte v255, v[0:1] offset:0 // FLAT
flat_load_sbyte v0, v[254:255] glc // FLAT
flat_load_dwordx2 v[254:255], v[0:1] slc // FLAT
flat_load_dwordx4 v[0:3], v[254:255] // FLAT
flat_store_dword v[254:255], v0 offset:2047 // FLAT
flat_store_short v[0:1], v255 offset:1 // FLAT
flat_atomic_add v255, v[0:1], v255 glc // FLAT
flat_atomic_swap v[254:255], v0 // FLAT
global_load_dword v255, v[0:1], off offset:-2048 // GLOBAL
global_load_dword v0, v255, s[104:105] offset:2047 // GLOBAL
global_load_ubyte v255, v0, s[0:1] // GLOBAL
global_load_dwordx3 v[253:255], v[0:1], off glc // GLOBAL
global_store_dword v255, v0, s[104:105] offset:-2048 // GLOBAL
global_store_dwordx4 v[254:255], v[0:3], off offset:2047 // GLOBAL
global_store_byte v0, v255, s[0:1] // GLOBAL
global_atomic_or v255, v0, v255, s[104:105] glc // GLOBAL
global_atomic_add v[254:255], v0, off // GLOBAL
scratch_load_dword v255, v0, off offset:2047 // SCRATCH
scratch_load_dword v0, off, s105 offset:-2048 // SCRATCH
scratch_load_ushort v255, v0, off // SCRATCH
scratch_load_dwordx2 v[254:255], off, s0 // SCRATCH
scratch_load_dwordx4 v[0:3], v255, off // SCRATCH
scratch_store_dword v255, v0, off offset:-2048 // SCRATCH
scratch_store_dword off, v255, s0 offset:2047 // SCRATCH
scratch_store_byte v0, v255, off // SCRATCH
scratch_store_dwordx4 off, v[252:255], s105 // SCRATCH
// Forms the byte probes could not reach with legal operands (FLAT SADDR, DS GDS, BVH).
ds_gws_init v1 offset:65535 gds // DS
ds_gws_barrier v1 offset:0 gds // DS
ds_gws_sema_v gds // DS
ds_gws_sema_br v1 gds // DS
ds_gws_sema_p gds // DS
ds_gws_sema_release_all gds // DS
ds_ordered_count v0, v1 offset:65535 gds // DS
flat_load_dwordx3 v[0:2], v[254:255] offset:2047 // FLAT
flat_load_sshort v255, v[254:255] offset:2047 // FLAT
flat_load_ushort v0, v[254:255] offset:2047 // FLAT
flat_load_sbyte_d16 v0, v[254:255] offset:2047 // FLAT
flat_load_sbyte_d16_hi v0, v[254:255] offset:2047 // FLAT
flat_load_short_d16 v0, v[254:255] offset:2047 // FLAT
flat_load_short_d16_hi v0, v[254:255] offset:2047 // FLAT
flat_load_ubyte_d16 v0, v[254:255] offset:2047 // FLAT
flat_load_ubyte_d16_hi v0, v[254:255] offset:2047 // FLAT
flat_store_byte v[0:1], v2 // FLAT
flat_store_byte_d16_hi v[0:1], v2 // FLAT
flat_store_dwordx2 v[0:1], v[2:3] // FLAT
flat_store_dwordx3 v[0:1], v[2:4] // FLAT
flat_store_dwordx4 v[0:1], v[2:5] // FLAT
flat_store_short_d16_hi v[0:1], v2 // FLAT
flat_atomic_add v0, v[0:1], v2 glc // FLAT
flat_atomic_add_x2 v[0:1], v[2:3] // FLAT
flat_atomic_and v0, v[0:1], v2 glc // FLAT
flat_atomic_and_x2 v[0:1], v[2:3] // FLAT
flat_atomic_dec v0, v[0:1], v2 glc // FLAT
flat_atomic_dec_x2 v[0:1], v[2:3] // FLAT
flat_atomic_inc v0, v[0:1], v2 glc // FLAT
flat_atomic_inc_x2 v[0:1], v[2:3] // FLAT
flat_atomic_or v0, v[0:1], v2 glc // FLAT
flat_atomic_or_x2 v[0:1], v[2:3] // FLAT
flat_atomic_smax v0, v[0:1], v2 glc // FLAT
flat_atomic_smax_x2 v[0:1], v[2:3] // FLAT
flat_atomic_smin v0, v[0:1], v2 glc // FLAT
flat_atomic_smin_x2 v[0:1], v[2:3] // FLAT
flat_atomic_sub v0, v[0:1], v2 glc // FLAT
flat_atomic_sub_x2 v[0:1], v[2:3] // FLAT
flat_atomic_umax v0, v[0:1], v2 glc // FLAT
flat_atomic_umax_x2 v[0:1], v[2:3] // FLAT
flat_atomic_umin v0, v[0:1], v2 glc // FLAT
flat_atomic_umin_x2 v[0:1], v[2:3] // FLAT
flat_atomic_xor v0, v[0:1], v2 glc // FLAT
flat_atomic_xor_x2 v[0:1], v[2:3] // FLAT
flat_atomic_fmax v0, v[0:1], v2 glc // FLAT
flat_atomic_fmax_x2 v[0:1], v[2:3] // FLAT
flat_atomic_fmin v0, v[0:1], v2 glc // FLAT
flat_atomic_fmin_x2 v[0:1], v[2:3] // FLAT
flat_atomic_swap_x2 v[0:1], v[2:3] offset:2047 // FLAT
flat_atomic_cmpswap v0, v[0:1], v[2:3] glc // FLAT
flat_atomic_cmpswap_x2 v[0:1], v[2:5] // FLAT
flat_atomic_fcmpswap v[0:1], v[2:3] // FLAT
flat_atomic_fcmpswap_x2 v[0:1], v[2:5] // FLAT
image_bvh_intersect_ray v[0:3], v[0:10], s[0:3] // MIMG
image_bvh64_intersect_ray v[0:3], v[0:11], s[0:3] // MIMG
image_bvh_intersect_ray v[0:3], [v4, v5, v6, v7, v8, v9, v10, v11, v12, v13, v14], s[0:3] // MIMG
v_interp_p1ll_f16 v0, v1, attr0.x // VOP3
v_interp_p1lv_f16 v0, v1, attr0.x, v2 // VOP3
v_interp_p2_f16 v0, v1, attr0.x, v2 // VOP3
v_mqsad_u32_u8 v[0:3], v[2:3], v4, v[4:7] // VOP3
v_permlane16_b32 v0, v1, s2, s3 // VOP3
v_permlanex16_b32 v255, v1, s105, m0 op_sel:[1,1] // VOP3
v_swaprel_b32 v0, v1 // VOP1
