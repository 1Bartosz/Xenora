// SPDX-License-Identifier: GPL-3.0-or-later

//! Bounded x86-64 to ARM64 Dynamic Binary Translation (DBT) engine for Xenora.
//!
//! Compiles decoded x86-64 basic blocks to native Apple Silicon ARM64 machine code,
//! executes them in a W^X code cache, and manages host-service dispatch and fallbacks.

use crate::arm64::{Arm64Assembler, SP, X0, X1, X2, X4, X5, X6, X9, X10, X19, X20, X21, X29, X30};
use crate::cpu::{CpuState, ExecutionResult, ExitReason, Register};
use crate::jit::{JitError, JitMemory};
use crate::memory::GuestMemory;
use std::collections::HashMap;

const MAX_BLOCK_INSTRUCTIONS: usize = 16;

/// Registered guest host-service handler function.
pub type HostServiceFn =
    Box<dyn Fn(&mut CpuState, &mut GuestMemory) -> Result<(), ExitReason> + Send + Sync>;

/// Diagnostic execution counters for DBT runtime profiling.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DbtMetrics {
    pub blocks_compiled: usize,
    pub cache_hits: usize,
    pub cache_misses: usize,
    pub native_bytes_emitted: usize,
    pub dbt_dispatches: usize,
    pub instructions_executed: usize,
    pub interpreter_fallbacks: usize,
    pub service_calls: usize,
}

/// Dynamic Binary Translator and native execution runtime.
pub struct DbtEngine {
    jit: JitMemory,
    block_cache: HashMap<u64, *const u8>,
    limited_block_cache: HashMap<(u64, usize), *const u8>,
    assembler: Arm64Assembler,
    services: HashMap<u64, HostServiceFn>,
    pub import_registry: Option<crate::imports::ImportRegistry>,
    pub metrics: DbtMetrics,
}

impl DbtEngine {
    /// Create a new DBT engine with specified JIT cache capacity (e.g. 1MB).
    pub fn new(capacity: usize) -> Result<Self, JitError> {
        Ok(Self {
            jit: JitMemory::new(capacity)?,
            block_cache: HashMap::new(),
            limited_block_cache: HashMap::new(),
            assembler: Arm64Assembler::new(),
            services: HashMap::new(),
            import_registry: None,
            metrics: DbtMetrics::default(),
        })
    }

    /// Attach an import registry for automatic IAT trampoline dispatch.
    pub fn set_import_registry(&mut self, registry: crate::imports::ImportRegistry) {
        self.import_registry = Some(registry);
    }

    /// Register a host service at a synthetic guest virtual address.
    pub fn register_service<F>(&mut self, address: u64, handler: F)
    where
        F: Fn(&mut CpuState, &mut GuestMemory) -> Result<(), ExitReason> + Send + Sync + 'static,
    {
        self.services.insert(address, Box::new(handler));
    }

    /// Execute guest code starting at CPU RIP until HLT, budget exhaustion, or fault.
    pub fn run(
        &mut self,
        cpu: &mut CpuState,
        memory: &mut GuestMemory,
        max_instructions: usize,
    ) -> ExecutionResult {
        let mut instructions_count = 0;

        while instructions_count < max_instructions {
            // Check if current RIP is an attached import registry trampoline
            if let Some(registry) = &self.import_registry
                && registry.is_service_trampoline(cpu.rip)
            {
                self.metrics.service_calls += 1;
                if let Err(reason) = registry.dispatch(cpu.rip, cpu, memory) {
                    return ExecutionResult {
                        instructions_executed: instructions_count,
                        final_state: cpu.clone(),
                        exit_reason: reason,
                    };
                }
                instructions_count += 1;
                continue;
            }

            // Check if current RIP is a registered host service
            if let Some(service) = self.services.get(&cpu.rip) {
                self.metrics.service_calls += 1;
                if let Err(reason) = service(cpu, memory) {
                    return ExecutionResult {
                        instructions_executed: instructions_count,
                        final_state: cpu.clone(),
                        exit_reason: reason,
                    };
                }
                instructions_count += 1;
                continue;
            }

            // Look up or translate block at cpu.rip
            let block_limit = (max_instructions - instructions_count).min(MAX_BLOCK_INSTRUCTIONS);
            let block_fn_ptr = match self.get_or_compile_block(cpu.rip, memory, block_limit) {
                Ok(ptr) => ptr,
                Err(reason) => {
                    return ExecutionResult {
                        instructions_executed: instructions_count,
                        final_state: cpu.clone(),
                        exit_reason: reason,
                    };
                }
            };

            // Execute native ARM64 translated block!
            // Signature: extern "C" fn(cpu_state_ptr: *mut u64, guest_memory_ctx: *mut (GuestMemory, ExitReason)) -> u32
            // Returns: number of guest instructions executed in this block
            type BlockFn = unsafe extern "C" fn(*mut u64, *mut BlockContext) -> u32;
            let block_fn: BlockFn = unsafe { std::mem::transmute(block_fn_ptr) };

            let mut ctx = BlockContext {
                memory_ptr: memory as *mut GuestMemory,
                faulted: 0,
                exit_reason: None,
            };

            self.metrics.dbt_dispatches += 1;
            let cpu_raw_ptr = cpu as *mut CpuState as *mut u64;
            let insns_in_block = unsafe { block_fn(cpu_raw_ptr, &mut ctx) };
            instructions_count += insns_in_block as usize;
            self.metrics.instructions_executed += insns_in_block as usize;

            if let Some(exit) = ctx.exit_reason {
                return ExecutionResult {
                    instructions_executed: instructions_count,
                    final_state: cpu.clone(),
                    exit_reason: exit,
                };
            }
        }

        ExecutionResult {
            instructions_executed: instructions_count,
            final_state: cpu.clone(),
            exit_reason: ExitReason::StepLimitExceeded,
        }
    }

    fn get_or_compile_block(
        &mut self,
        rip: u64,
        memory: &GuestMemory,
        block_limit: usize,
    ) -> Result<*const u8, ExitReason> {
        if block_limit == MAX_BLOCK_INSTRUCTIONS {
            if let Some(&fn_ptr) = self.block_cache.get(&rip) {
                self.metrics.cache_hits += 1;
                return Ok(fn_ptr);
            }
            self.metrics.cache_misses += 1;
            let compiled_ptr = self.compile_block(rip, memory, block_limit)?;
            self.block_cache.insert(rip, compiled_ptr);
            return Ok(compiled_ptr);
        }

        // Keep budget-limited blocks separate so they cannot shorten a later run.
        let key = (rip, block_limit);
        if let Some(&fn_ptr) = self.limited_block_cache.get(&key) {
            self.metrics.cache_hits += 1;
            return Ok(fn_ptr);
        }
        self.metrics.cache_misses += 1;
        let compiled_ptr = self.compile_block(rip, memory, block_limit)?;
        self.limited_block_cache.insert(key, compiled_ptr);
        Ok(compiled_ptr)
    }

    fn compile_block(
        &mut self,
        start_rip: u64,
        memory: &GuestMemory,
        block_limit: usize,
    ) -> Result<*const u8, ExitReason> {
        // Reuse the emission buffer across cold block translations.
        let asm = &mut self.assembler;
        asm.clear();
        let mut cur_rip = start_rip;
        let mut insn_count = 0;
        // Branch instructions to patch once the shared epilogue offset is known.
        let mut fault_guards: Vec<usize> = Vec::new();

        // Function Prologue:
        // Inputs: X0 = &mut CpuState.gpr[0], X1 = &mut BlockContext
        // We preserve host callee-saved registers X19, X20, X21, X29 (FP), X30 (LR)
        asm.stp_pre_64(X29, X30, SP, -48);
        asm.str_uoff_64(X19, SP, 16);
        asm.str_uoff_64(X20, SP, 24);
        asm.str_uoff_64(X21, SP, 32);

        // Keep CPU GPR pointer in callee-saved X19
        asm.mov_reg_64(X19, X0);
        // Keep BlockContext pointer in callee-saved X20
        asm.mov_reg_64(X20, X1);

        // Stop at the instruction quota or a control-flow boundary.
        loop {
            let insn_rip = cur_rip;
            let b0 = memory
                .fetch_instruction_byte(cur_rip)
                .map_err(ExitReason::MemoryFault)?;
            cur_rip += 1;

            let mut rex_w = false;
            let mut rex_r = false;
            let mut rex_x = false;
            let mut rex_b = false;
            let mut op = b0;

            if (op & 0xf0) == 0x40 {
                rex_w = (op & 0x08) != 0;
                rex_r = (op & 0x04) != 0;
                rex_x = (op & 0x02) != 0;
                rex_b = (op & 0x01) != 0;
                op = memory
                    .fetch_instruction_byte(cur_rip)
                    .map_err(ExitReason::MemoryFault)?;
                cur_rip += 1;
            }

            match op {
                // MOV r/m, imm32 (0xC7 /0)
                0xC7 => {
                    insn_count += 1;
                    let modrm = memory
                        .fetch_instruction_byte(cur_rip)
                        .map_err(ExitReason::MemoryFault)?;
                    cur_rip += 1;
                    let sub_op = (modrm >> 3) & 7;
                    let mode = (modrm >> 6) & 3;
                    if sub_op != 0 {
                        return Err(ExitReason::IllegalInstruction(op));
                    }

                    if mode == 3 {
                        let reg_idx = (modrm & 7) + if rex_b { 8 } else { 0 };
                        let mut imm_bytes = [0u8; 4];
                        for b in &mut imm_bytes {
                            *b = memory
                                .fetch_instruction_byte(cur_rip)
                                .map_err(ExitReason::MemoryFault)?;
                            cur_rip += 1;
                        }
                        let imm32 = i32::from_le_bytes(imm_bytes) as i64;
                        let imm_val = if rex_w {
                            imm32 as u64
                        } else {
                            (imm32 as u32) as u64
                        };
                        asm.load_imm_64(X9, imm_val);
                        asm.str_uoff_64(X9, X19, (reg_idx as u32) * 8);
                    } else if mode == 0 {
                        // [reg] without displacement or [RIP + disp32]
                        if (modrm & 7) == 5 {
                            // [RIP + disp32]
                            let mut disp_bytes = [0u8; 4];
                            for b in &mut disp_bytes {
                                *b = memory
                                    .fetch_instruction_byte(cur_rip)
                                    .map_err(ExitReason::MemoryFault)?;
                                cur_rip += 1;
                            }
                            let disp32 = i32::from_le_bytes(disp_bytes) as i64;
                            let mut imm_bytes = [0u8; 4];
                            for b in &mut imm_bytes {
                                *b = memory
                                    .fetch_instruction_byte(cur_rip)
                                    .map_err(ExitReason::MemoryFault)?;
                                cur_rip += 1;
                            }
                            let target_ea = cur_rip.wrapping_add_signed(disp32);
                            let imm32 = i32::from_le_bytes(imm_bytes) as i64;
                            let imm_val = if rex_w {
                                imm32 as u64
                            } else {
                                (imm32 as u32) as u64
                            };

                            asm.mov_reg_64(X0, X20);
                            asm.load_imm_64(X1, target_ea);
                            asm.load_imm_64(X2, imm_val);
                            let helper_fn = write_u64_helper as *const () as u64;
                            asm.load_imm_64(X4, helper_fn);
                            asm.blr(X4);
                            emit_fault_guard(asm, &mut fault_guards, insn_count - 1, insn_rip);
                        } else if (modrm & 7) != 4 {
                            // [reg]
                            let reg_idx = (modrm & 7) + if rex_b { 8 } else { 0 };
                            let mut imm_bytes = [0u8; 4];
                            for b in &mut imm_bytes {
                                *b = memory
                                    .fetch_instruction_byte(cur_rip)
                                    .map_err(ExitReason::MemoryFault)?;
                                cur_rip += 1;
                            }
                            let imm32 = i32::from_le_bytes(imm_bytes) as i64;
                            let imm_val = if rex_w {
                                imm32 as u64
                            } else {
                                (imm32 as u32) as u64
                            };

                            asm.ldr_uoff_64(X1, X19, (reg_idx as u32) * 8); // target EA in X1
                            asm.mov_reg_64(X0, X20);
                            asm.load_imm_64(X2, imm_val);
                            let helper_fn = write_u64_helper as *const () as u64;
                            asm.load_imm_64(X4, helper_fn);
                            asm.blr(X4);
                            emit_fault_guard(asm, &mut fault_guards, insn_count - 1, insn_rip);
                        } else {
                            return Err(ExitReason::IllegalInstruction(op));
                        }
                    } else {
                        return Err(ExitReason::IllegalInstruction(op));
                    }
                }

                // HLT (0xF4): Terminate block with ExitReason::Halt
                0xF4 => {
                    insn_count += 1;
                    Self::emit_set_exit_reason(asm, 1, cur_rip); // 1 = Halt
                    break;
                }

                // MOV r32, imm32 (0xB8 + reg) or MOV r64, imm64 (with REX.W)
                0xB8..=0xBF => {
                    insn_count += 1;
                    let reg_idx = (op - 0xB8) + if rex_b { 8 } else { 0 };
                    let imm = if rex_w {
                        let mut buf = [0u8; 8];
                        for b in &mut buf {
                            *b = memory
                                .fetch_instruction_byte(cur_rip)
                                .map_err(ExitReason::MemoryFault)?;
                            cur_rip += 1;
                        }
                        u64::from_le_bytes(buf)
                    } else {
                        let mut buf = [0u8; 4];
                        for b in &mut buf {
                            *b = memory
                                .fetch_instruction_byte(cur_rip)
                                .map_err(ExitReason::MemoryFault)?;
                            cur_rip += 1;
                        }
                        u32::from_le_bytes(buf) as u64
                    };

                    asm.load_imm_64(X9, imm);
                    asm.str_uoff_64(X9, X19, (reg_idx as u32) * 8);
                }

                // ADD r/m, reg (0x01) or ADD reg, r/m (0x03)
                0x01 | 0x03 => {
                    insn_count += 1;
                    let modrm = memory
                        .fetch_instruction_byte(cur_rip)
                        .map_err(ExitReason::MemoryFault)?;
                    cur_rip += 1;
                    let mode = (modrm >> 6) & 3;
                    if mode != 3 {
                        return Err(ExitReason::IllegalInstruction(op));
                    }
                    let reg_idx = ((modrm >> 3) & 7) + if rex_r { 8 } else { 0 };
                    let rm_idx = (modrm & 7) + if rex_b { 8 } else { 0 };
                    let (src, dst) = if op == 0x01 {
                        (reg_idx, rm_idx)
                    } else {
                        (rm_idx, reg_idx)
                    };

                    asm.ldr_uoff_64(X9, X19, (dst as u32) * 8);
                    asm.ldr_uoff_64(X10, X19, (src as u32) * 8);
                    if rex_w {
                        asm.add_reg_64(X9, X9, X10);
                    } else {
                        asm.add_reg_32(X9, X9, X10); // W9 = W9 + W10 (automatically zero-extended to X9 in AArch64)
                    }
                    asm.str_uoff_64(X9, X19, (dst as u32) * 8);
                }

                // SUB r/m, reg (0x29) or SUB reg, r/m (0x2B)
                0x29 | 0x2B => {
                    insn_count += 1;
                    let modrm = memory
                        .fetch_instruction_byte(cur_rip)
                        .map_err(ExitReason::MemoryFault)?;
                    cur_rip += 1;
                    let mode = (modrm >> 6) & 3;
                    if mode != 3 {
                        return Err(ExitReason::IllegalInstruction(op));
                    }
                    let reg_idx = ((modrm >> 3) & 7) + if rex_r { 8 } else { 0 };
                    let rm_idx = (modrm & 7) + if rex_b { 8 } else { 0 };
                    let (src, dst) = if op == 0x29 {
                        (reg_idx, rm_idx)
                    } else {
                        (rm_idx, reg_idx)
                    };

                    asm.ldr_uoff_64(X9, X19, (dst as u32) * 8);
                    asm.ldr_uoff_64(X10, X19, (src as u32) * 8);
                    if rex_w {
                        asm.sub_reg_64(X9, X9, X10);
                    } else {
                        asm.sub_reg_32(X9, X9, X10);
                    }
                    asm.str_uoff_64(X9, X19, (dst as u32) * 8);
                }

                // XOR r/m, reg (0x31) or XOR reg, r/m (0x33)
                0x31 | 0x33 => {
                    insn_count += 1;
                    let modrm = memory
                        .fetch_instruction_byte(cur_rip)
                        .map_err(ExitReason::MemoryFault)?;
                    cur_rip += 1;
                    let mode = (modrm >> 6) & 3;
                    if mode != 3 {
                        return Err(ExitReason::IllegalInstruction(op));
                    }
                    let reg_idx = ((modrm >> 3) & 7) + if rex_r { 8 } else { 0 };
                    let rm_idx = (modrm & 7) + if rex_b { 8 } else { 0 };
                    let (src, dst) = if op == 0x31 {
                        (reg_idx, rm_idx)
                    } else {
                        (rm_idx, reg_idx)
                    };

                    asm.ldr_uoff_64(X9, X19, (dst as u32) * 8);
                    asm.ldr_uoff_64(X10, X19, (src as u32) * 8);
                    asm.eor_reg_64(X9, X9, X10);
                    if !rex_w {
                        asm.mov_reg_32(X9, X9); // zero extend
                    }
                    asm.str_uoff_64(X9, X19, (dst as u32) * 8);
                }

                // Imm8 group (0x83): ADD/SUB reg, imm8
                0x83 => {
                    insn_count += 1;
                    let modrm = memory
                        .fetch_instruction_byte(cur_rip)
                        .map_err(ExitReason::MemoryFault)?;
                    cur_rip += 1;
                    let mode = (modrm >> 6) & 3;
                    if mode != 3 {
                        return Err(ExitReason::IllegalInstruction(op));
                    }
                    let sub_op = (modrm >> 3) & 7;
                    let rm_idx = (modrm & 7) + if rex_b { 8 } else { 0 };
                    let imm8 = memory
                        .fetch_instruction_byte(cur_rip)
                        .map_err(ExitReason::MemoryFault)? as i8;
                    cur_rip += 1;

                    if !matches!(sub_op, 0 | 5) {
                        return Err(ExitReason::IllegalInstruction(op));
                    }
                    let reg_offset = u32::from(rm_idx) * 8;
                    let subtract = sub_op == 5;
                    asm.ldr_uoff_64(X9, X19, reg_offset);
                    if rex_w {
                        // A signed imm8 always fits the existing unsigned
                        // immediate forms after choosing ADD or SUB by sign.
                        let magnitude = u16::from(imm8.unsigned_abs());
                        if subtract ^ (imm8 < 0) {
                            asm.sub_imm_64(X9, X9, magnitude);
                        } else {
                            asm.add_imm_64(X9, X9, magnitude);
                        }
                    } else {
                        asm.load_imm_64(X10, (imm8 as i64) as u64);
                        if subtract {
                            asm.sub_reg_32(X9, X9, X10);
                        } else {
                            asm.add_reg_32(X9, X9, X10);
                        }
                    }
                    asm.str_uoff_64(X9, X19, reg_offset);
                }

                // PUSH reg (0x50..=0x57)
                0x50..=0x57 => {
                    insn_count += 1;
                    let reg_idx = (op - 0x50) + if rex_b { 8 } else { 0 };
                    // 1. Load reg value into X9
                    asm.ldr_uoff_64(X9, X19, (reg_idx as u32) * 8);
                    // 2. Load RSP (index 4) into X10, subtract 8
                    asm.ldr_uoff_64(X10, X19, (Register::Rsp as u32) * 8);
                    asm.sub_imm_64(X10, X10, 8);
                    // 3. Call helper to write memory: write_u64_helper(ctx: X0, addr: X1, val: X2)
                    asm.mov_reg_64(X0, X20);
                    asm.mov_reg_64(X1, X10);
                    asm.mov_reg_64(X2, X9);
                    let helper_fn = write_u64_helper as *const () as u64;
                    asm.load_imm_64(X4, helper_fn);
                    asm.blr(X4);
                    emit_fault_guard(asm, &mut fault_guards, insn_count - 1, insn_rip);
                    // 4. Commit RSP only after the store succeeded. Re-read the
                    // guest RSP because the helper may clobber caller-saved X10.
                    asm.ldr_uoff_64(X10, X19, (Register::Rsp as u32) * 8);
                    asm.sub_imm_64(X10, X10, 8);
                    asm.str_uoff_64(X10, X19, (Register::Rsp as u32) * 8);
                }

                // POP reg (0x58..=0x5F)
                0x58..=0x5F => {
                    insn_count += 1;
                    let reg_idx = (op - 0x58) + if rex_b { 8 } else { 0 };
                    // 1. Load RSP into X1
                    asm.ldr_uoff_64(X1, X19, (Register::Rsp as u32) * 8);
                    // 2. Read memory: read_u64_helper(ctx: X0, addr: X1) -> X0
                    asm.mov_reg_64(X0, X20);
                    let helper_fn = read_u64_helper as *const () as u64;
                    asm.load_imm_64(X4, helper_fn);
                    asm.blr(X4);
                    emit_fault_guard(asm, &mut fault_guards, insn_count - 1, insn_rip);
                    // 3. Result in X0 -> store to reg_idx
                    asm.str_uoff_64(X0, X19, (reg_idx as u32) * 8);
                    // 4. Update RSP += 8
                    asm.ldr_uoff_64(X10, X19, (Register::Rsp as u32) * 8);
                    asm.add_imm_64(X10, X10, 8);
                    asm.str_uoff_64(X10, X19, (Register::Rsp as u32) * 8);
                }

                // CALL rel32 (0xE8)
                0xE8 => {
                    insn_count += 1;
                    let mut rel_bytes = [0u8; 4];
                    for b in &mut rel_bytes {
                        *b = memory
                            .fetch_instruction_byte(cur_rip)
                            .map_err(ExitReason::MemoryFault)?;
                        cur_rip += 1;
                    }
                    let rel32 = i32::from_le_bytes(rel_bytes) as i64;
                    let target_rip = (cur_rip as i64 + rel32) as u64;
                    let ret_addr = cur_rip;

                    // Push ret_addr to guest stack
                    asm.load_imm_64(X9, ret_addr);
                    asm.ldr_uoff_64(X10, X19, (Register::Rsp as u32) * 8);
                    asm.sub_imm_64(X10, X10, 8);
                    asm.mov_reg_64(X0, X20);
                    asm.mov_reg_64(X1, X10);
                    asm.mov_reg_64(X2, X9);
                    let helper_fn = write_u64_helper as *const () as u64;
                    asm.load_imm_64(X4, helper_fn);
                    asm.blr(X4);
                    emit_fault_guard(asm, &mut fault_guards, insn_count - 1, insn_rip);
                    // Commit RSP only after the return address was written. The
                    // helper may clobber caller-saved registers, so recompute it.
                    asm.ldr_uoff_64(X10, X19, (Register::Rsp as u32) * 8);
                    asm.sub_imm_64(X10, X10, 8);
                    asm.str_uoff_64(X10, X19, (Register::Rsp as u32) * 8);

                    // Update RIP to target_rip and finish block
                    asm.load_imm_64(X9, target_rip);
                    asm.str_uoff_64(X9, X19, 16 * 8); // RIP is offset 16 * 8
                    break;
                }

                // RET (0xC3)
                0xC3 => {
                    insn_count += 1;
                    // Read return address from [RSP]
                    asm.ldr_uoff_64(X1, X19, (Register::Rsp as u32) * 8);
                    asm.mov_reg_64(X0, X20);
                    let helper_fn = read_u64_helper as *const () as u64;
                    asm.load_imm_64(X4, helper_fn);
                    asm.blr(X4);
                    emit_fault_guard(asm, &mut fault_guards, insn_count - 1, insn_rip);
                    // Update RIP = return address
                    asm.str_uoff_64(X0, X19, 16 * 8);
                    // RSP += 8
                    asm.ldr_uoff_64(X10, X19, (Register::Rsp as u32) * 8);
                    asm.add_imm_64(X10, X10, 8);
                    asm.str_uoff_64(X10, X19, (Register::Rsp as u32) * 8);
                    break;
                }

                // JMP rel32 (0xE9)
                0xE9 => {
                    insn_count += 1;
                    let mut rel_bytes = [0u8; 4];
                    for b in &mut rel_bytes {
                        *b = memory
                            .fetch_instruction_byte(cur_rip)
                            .map_err(ExitReason::MemoryFault)?;
                        cur_rip += 1;
                    }
                    let rel32 = i32::from_le_bytes(rel_bytes) as i64;
                    let target_rip = (cur_rip as i64 + rel32) as u64;
                    asm.load_imm_64(X9, target_rip);
                    asm.str_uoff_64(X9, X19, 16 * 8);
                    break;
                }

                // JMP rel8 (0xEB)
                0xEB => {
                    insn_count += 1;
                    let rel8 = memory
                        .fetch_instruction_byte(cur_rip)
                        .map_err(ExitReason::MemoryFault)? as i8;
                    cur_rip += 1;
                    let target_rip = (cur_rip as i64 + rel8 as i64) as u64;
                    asm.load_imm_64(X9, target_rip);
                    asm.str_uoff_64(X9, X19, 16 * 8);
                    break;
                }

                // MOV r/m, reg (0x89) or MOV reg, r/m (0x8B)
                0x89 | 0x8B => {
                    insn_count += 1;
                    let modrm = memory
                        .fetch_instruction_byte(cur_rip)
                        .map_err(ExitReason::MemoryFault)?;
                    cur_rip += 1;
                    let mode = (modrm >> 6) & 3;
                    let reg_idx = ((modrm >> 3) & 7) + if rex_r { 8 } else { 0 };
                    let rm_idx = (modrm & 7) + if rex_b { 8 } else { 0 };

                    if mode == 3 {
                        // Register-to-register move
                        let (src, dst) = if op == 0x89 {
                            (reg_idx, rm_idx)
                        } else {
                            (rm_idx, reg_idx)
                        };
                        asm.ldr_uoff_64(X9, X19, (src as u32) * 8);
                        if !rex_w {
                            asm.mov_reg_32(X9, X9); // zero-extend to 64-bit
                        }
                        asm.str_uoff_64(X9, X19, (dst as u32) * 8);
                    } else if mode == 0 && (modrm & 7) == 5 {
                        // RIP-relative: [RIP + disp32]
                        let mut disp_bytes = [0u8; 4];
                        for b in &mut disp_bytes {
                            *b = memory
                                .fetch_instruction_byte(cur_rip)
                                .map_err(ExitReason::MemoryFault)?;
                            cur_rip += 1;
                        }
                        let disp32 = i32::from_le_bytes(disp_bytes) as i64;
                        let target_ea = ((cur_rip as i64) + disp32) as u64;

                        if op == 0x8B {
                            // MOV reg, [RIP + disp32]
                            asm.mov_reg_64(X0, X20);
                            asm.load_imm_64(X1, target_ea);
                            let helper_fn = read_u64_helper as *const () as u64;
                            asm.load_imm_64(X4, helper_fn);
                            asm.blr(X4);
                            emit_fault_guard(asm, &mut fault_guards, insn_count - 1, insn_rip);
                            if !rex_w {
                                asm.mov_reg_32(X0, X0);
                            }
                            asm.str_uoff_64(X0, X19, (reg_idx as u32) * 8);
                        } else {
                            // MOV [RIP + disp32], reg
                            asm.ldr_uoff_64(X2, X19, (reg_idx as u32) * 8);
                            asm.mov_reg_64(X0, X20);
                            asm.load_imm_64(X1, target_ea);
                            let helper_fn = write_u64_helper as *const () as u64;
                            asm.load_imm_64(X4, helper_fn);
                            asm.blr(X4);
                            emit_fault_guard(asm, &mut fault_guards, insn_count - 1, insn_rip);
                        }
                    } else {
                        // Memory operand with base register and optional SIB / displacement
                        // Handle SIB if (modrm & 7) == 4
                        let base_reg = if (modrm & 7) == 4 {
                            let sib = memory
                                .fetch_instruction_byte(cur_rip)
                                .map_err(ExitReason::MemoryFault)?;
                            cur_rip += 1;
                            if rex_x || (sib >> 3) & 7 != 4 || (mode == 0 && sib & 7 == 5) {
                                return Err(ExitReason::IllegalInstruction(op));
                            }
                            (sib & 7) + if rex_b { 8 } else { 0 }
                        } else {
                            rm_idx
                        };

                        let disp = match mode {
                            1 => {
                                let d = memory
                                    .fetch_instruction_byte(cur_rip)
                                    .map_err(ExitReason::MemoryFault)?
                                    as i8;
                                cur_rip += 1;
                                d as i64
                            }
                            2 => {
                                let mut d_bytes = [0u8; 4];
                                for b in &mut d_bytes {
                                    *b = memory
                                        .fetch_instruction_byte(cur_rip)
                                        .map_err(ExitReason::MemoryFault)?;
                                    cur_rip += 1;
                                }
                                i32::from_le_bytes(d_bytes) as i64
                            }
                            _ => 0i64,
                        };

                        // Compute effective address in X1: base_reg + disp
                        asm.ldr_uoff_64(X1, X19, (base_reg as u32) * 8);
                        if disp != 0 {
                            if disp > 0 && disp <= 4095 {
                                asm.add_imm_64(X1, X1, disp as u16);
                            } else if disp < 0 && (-disp) <= 4095 {
                                asm.sub_imm_64(X1, X1, (-disp) as u16);
                            } else {
                                asm.load_imm_64(X9, disp as u64);
                                asm.add_reg_64(X1, X1, X9);
                            }
                        }

                        if op == 0x89 {
                            // MOV [EA], reg: load reg into X2 and call write_u64_helper(ctx: X0, addr: X1, val: X2)
                            asm.ldr_uoff_64(X2, X19, (reg_idx as u32) * 8);
                            asm.mov_reg_64(X0, X20);
                            let helper_fn = write_u64_helper as *const () as u64;
                            asm.load_imm_64(X4, helper_fn);
                            asm.blr(X4);
                            emit_fault_guard(asm, &mut fault_guards, insn_count - 1, insn_rip);
                        } else {
                            // MOV reg, [EA]: call read_u64_helper(ctx: X0, addr: X1) -> X0
                            asm.mov_reg_64(X0, X20);
                            let helper_fn = read_u64_helper as *const () as u64;
                            asm.load_imm_64(X4, helper_fn);
                            asm.blr(X4);
                            emit_fault_guard(asm, &mut fault_guards, insn_count - 1, insn_rip);
                            if !rex_w {
                                asm.mov_reg_32(X0, X0);
                            }
                            asm.str_uoff_64(X0, X19, (reg_idx as u32) * 8);
                        }
                    }
                }

                // LEA reg, [EA] (0x8D)
                0x8D => {
                    insn_count += 1;
                    let modrm = memory
                        .fetch_instruction_byte(cur_rip)
                        .map_err(ExitReason::MemoryFault)?;
                    cur_rip += 1;
                    let mode = (modrm >> 6) & 3;
                    let reg_idx = ((modrm >> 3) & 7) + if rex_r { 8 } else { 0 };
                    let rm_idx = (modrm & 7) + if rex_b { 8 } else { 0 };
                    if mode == 3 {
                        return Err(ExitReason::IllegalInstruction(op));
                    }

                    if mode == 0 && (modrm & 7) == 5 {
                        // RIP-relative LEA: [RIP + disp32]
                        let mut disp_bytes = [0u8; 4];
                        for b in &mut disp_bytes {
                            *b = memory
                                .fetch_instruction_byte(cur_rip)
                                .map_err(ExitReason::MemoryFault)?;
                            cur_rip += 1;
                        }
                        let disp32 = i32::from_le_bytes(disp_bytes) as i64;
                        let target_ea = ((cur_rip as i64) + disp32) as u64;
                        asm.load_imm_64(X9, target_ea);
                        if !rex_w {
                            asm.mov_reg_32(X9, X9);
                        }
                        asm.str_uoff_64(X9, X19, (reg_idx as u32) * 8);
                    } else {
                        let base_reg = if (modrm & 7) == 4 {
                            let sib = memory
                                .fetch_instruction_byte(cur_rip)
                                .map_err(ExitReason::MemoryFault)?;
                            cur_rip += 1;
                            if rex_x || (sib >> 3) & 7 != 4 || (mode == 0 && sib & 7 == 5) {
                                return Err(ExitReason::IllegalInstruction(op));
                            }
                            (sib & 7) + if rex_b { 8 } else { 0 }
                        } else {
                            rm_idx
                        };

                        let disp = match mode {
                            1 => {
                                let d = memory
                                    .fetch_instruction_byte(cur_rip)
                                    .map_err(ExitReason::MemoryFault)?
                                    as i8;
                                cur_rip += 1;
                                d as i64
                            }
                            2 => {
                                let mut d_bytes = [0u8; 4];
                                for b in &mut d_bytes {
                                    *b = memory
                                        .fetch_instruction_byte(cur_rip)
                                        .map_err(ExitReason::MemoryFault)?;
                                    cur_rip += 1;
                                }
                                i32::from_le_bytes(d_bytes) as i64
                            }
                            _ => 0i64,
                        };

                        asm.ldr_uoff_64(X9, X19, (base_reg as u32) * 8);
                        if disp != 0 {
                            if disp > 0 && disp <= 4095 {
                                asm.add_imm_64(X9, X9, disp as u16);
                            } else if disp < 0 && (-disp) <= 4095 {
                                asm.sub_imm_64(X9, X9, (-disp) as u16);
                            } else {
                                asm.load_imm_64(X10, disp as u64);
                                asm.add_reg_64(X9, X9, X10);
                            }
                        }
                        if !rex_w {
                            asm.mov_reg_32(X9, X9);
                        }
                        asm.str_uoff_64(X9, X19, (reg_idx as u32) * 8);
                    }
                }

                // Group 5: CALL/JMP indirect (0xFF)
                0xFF => {
                    insn_count += 1;
                    let modrm = memory
                        .fetch_instruction_byte(cur_rip)
                        .map_err(ExitReason::MemoryFault)?;
                    cur_rip += 1;
                    let sub_op = (modrm >> 3) & 7;
                    let mode = (modrm >> 6) & 3;

                    if sub_op <= 1 {
                        // Only the 64-bit register forms are supported.
                        if mode != 3 || !rex_w {
                            return Err(ExitReason::IllegalInstruction(op));
                        }
                        let reg_idx = (modrm & 7) + if rex_b { 8 } else { 0 };
                        asm.ldr_uoff_64(X9, X19, (reg_idx as u32) * 8);
                        if sub_op == 0 {
                            asm.add_imm_64(X9, X9, 1);
                        } else {
                            asm.sub_imm_64(X9, X9, 1);
                        }
                        asm.str_uoff_64(X9, X19, (reg_idx as u32) * 8);
                    } else if sub_op == 2 {
                        // CALL near indirect (call *[EA] or call *reg)
                        let _ret_addr = cur_rip + if mode == 0 && (modrm & 7) == 5 { 4 } else { 0 };

                        if mode == 0 && (modrm & 7) == 5 {
                            // call qword ptr [rip + disp32] (standard IAT call form)
                            let mut disp_bytes = [0u8; 4];
                            for b in &mut disp_bytes {
                                *b = memory
                                    .fetch_instruction_byte(cur_rip)
                                    .map_err(ExitReason::MemoryFault)?;
                                cur_rip += 1;
                            }
                            let disp32 = i32::from_le_bytes(disp_bytes) as i64;
                            let target_ea = ((cur_rip as i64) + disp32) as u64;

                            // x86 reads the indirect target before pushing the
                            // return address, so a faulting IAT read must leave
                            // the guest stack untouched.
                            asm.mov_reg_64(X0, X20);
                            asm.load_imm_64(X1, target_ea);
                            let helper_fn = read_u64_helper as *const () as u64;
                            asm.load_imm_64(X4, helper_fn);
                            asm.blr(X4);
                            emit_fault_guard(asm, &mut fault_guards, insn_count - 1, insn_rip);
                            // Keep the target in a callee-saved register because
                            // the stack-write helper may clobber caller-saved X9.
                            asm.mov_reg_64(X21, X0);

                            // Push return address to guest stack
                            asm.load_imm_64(X10, cur_rip);
                            asm.ldr_uoff_64(X1, X19, (Register::Rsp as u32) * 8);
                            asm.sub_imm_64(X1, X1, 8);
                            asm.mov_reg_64(X0, X20);
                            asm.mov_reg_64(X2, X10);
                            let write_fn = write_u64_helper as *const () as u64;
                            asm.load_imm_64(X4, write_fn);
                            asm.blr(X4);
                            emit_fault_guard(asm, &mut fault_guards, insn_count - 1, insn_rip);
                            // Commit RSP only after the return address was
                            // written; the helper clobbers caller-saved X1.
                            asm.ldr_uoff_64(X10, X19, (Register::Rsp as u32) * 8);
                            asm.sub_imm_64(X10, X10, 8);
                            asm.str_uoff_64(X10, X19, (Register::Rsp as u32) * 8);

                            // Set RIP = target and end the block.
                            asm.str_uoff_64(X21, X19, 16 * 8);
                            break;
                        } else if mode == 3 {
                            // call *reg
                            let reg_idx = (modrm & 7) + if rex_b { 8 } else { 0 };
                            asm.ldr_uoff_64(X21, X19, (reg_idx as u32) * 8); // target

                            // Push return address to stack
                            asm.load_imm_64(X10, cur_rip);
                            asm.ldr_uoff_64(X1, X19, (Register::Rsp as u32) * 8);
                            asm.sub_imm_64(X1, X1, 8);
                            asm.mov_reg_64(X0, X20);
                            asm.mov_reg_64(X2, X10);
                            let write_fn = write_u64_helper as *const () as u64;
                            asm.load_imm_64(X4, write_fn);
                            asm.blr(X4);
                            emit_fault_guard(asm, &mut fault_guards, insn_count - 1, insn_rip);
                            // Commit RSP only after the return address was
                            // written; the helper clobbers caller-saved X1.
                            asm.ldr_uoff_64(X10, X19, (Register::Rsp as u32) * 8);
                            asm.sub_imm_64(X10, X10, 8);
                            asm.str_uoff_64(X10, X19, (Register::Rsp as u32) * 8);

                            // Set RIP = target
                            asm.str_uoff_64(X21, X19, 16 * 8);
                            break;
                        } else {
                            return Err(ExitReason::IllegalInstruction(op));
                        }
                    } else if sub_op == 4 {
                        // JMP near indirect (jmp *[EA] or jmp *reg)
                        if mode == 3 {
                            let reg_idx = (modrm & 7) + if rex_b { 8 } else { 0 };
                            asm.ldr_uoff_64(X9, X19, (reg_idx as u32) * 8);
                            asm.str_uoff_64(X9, X19, 16 * 8);
                            break;
                        } else if mode == 0 && (modrm & 7) == 5 {
                            let mut disp_bytes = [0u8; 4];
                            for b in &mut disp_bytes {
                                *b = memory
                                    .fetch_instruction_byte(cur_rip)
                                    .map_err(ExitReason::MemoryFault)?;
                                cur_rip += 1;
                            }
                            let disp32 = i32::from_le_bytes(disp_bytes) as i64;
                            let target_ea = ((cur_rip as i64) + disp32) as u64;

                            asm.mov_reg_64(X0, X20);
                            asm.load_imm_64(X1, target_ea);
                            let helper_fn = read_u64_helper as *const () as u64;
                            asm.load_imm_64(X4, helper_fn);
                            asm.blr(X4);
                            emit_fault_guard(asm, &mut fault_guards, insn_count - 1, insn_rip);

                            asm.str_uoff_64(X0, X19, 16 * 8);
                            break;
                        } else {
                            return Err(ExitReason::IllegalInstruction(op));
                        }
                    } else {
                        return Err(ExitReason::IllegalInstruction(op));
                    }
                }

                _ => {
                    let _ = insn_rip;
                    return Err(ExitReason::IllegalInstruction(op));
                }
            }

            if insn_count >= block_limit {
                // End of basic block quota, update RIP and break
                asm.load_imm_64(X9, cur_rip);
                asm.str_uoff_64(X9, X19, 16 * 8);
                break;
            }
        }

        // Return number of guest instructions executed in this block in X0
        asm.load_imm_64(X0, insn_count as u64);

        // Epilogue: faulting paths jump here with X0 already set to the number
        // of instructions that completed before the fault.
        let epilogue = asm.current_offset();
        for index in fault_guards {
            asm.patch_b(index, epilogue as i32 - index as i32);
        }
        asm.ldr_uoff_64(X19, SP, 16);
        asm.ldr_uoff_64(X20, SP, 24);
        asm.ldr_uoff_64(X21, SP, 32);
        asm.ldp_post_64(X29, X30, SP, 48);
        asm.ret();

        let emitted = self
            .jit
            .emit_block(asm.instructions())
            .map_err(|_| ExitReason::JitCacheExhausted)?;
        self.metrics.blocks_compiled += 1;
        self.metrics.native_bytes_emitted += asm.instructions().len() * 4;
        Ok(emitted)
    }

    fn emit_set_exit_reason(asm: &mut Arm64Assembler, code: u32, next_rip: u64) {
        asm.load_imm_64(X9, next_rip);
        asm.str_uoff_64(X9, X19, 16 * 8); // Update RIP

        // Call helper to set exit reason
        asm.mov_reg_64(X0, X20); // X0 = BlockContext
        asm.load_imm_64(X1, code as u64); // X1 = code (1 = Halt)
        let set_exit_fn = set_exit_reason_helper as *const () as u64;
        asm.load_imm_64(X4, set_exit_fn);
        asm.blr(X4);
    }
}

/// Emit the post-helper fault check for a translated memory access.
///
/// The helper records a fault in the block context instead of unwinding through
/// the native frame. This guard lets translated code observe that flag, restore
/// nothing (no architectural effect may have been committed yet), report the
/// number of fully executed instructions, point guest RIP at the faulting
/// instruction, and jump to the shared epilogue.
fn emit_fault_guard(
    asm: &mut Arm64Assembler,
    guards: &mut Vec<usize>,
    executed_before: usize,
    fault_rip: u64,
) {
    asm.ldr_uoff_64(X5, X20, BLOCK_CTX_FAULTED_OFFSET);
    let skip = asm.current_offset();
    asm.cbz_64(X5, 0);
    asm.load_imm_64(X6, fault_rip);
    asm.str_uoff_64(X6, X19, 16 * 8); // Guest RIP lives at byte offset 128.
    asm.load_imm_64(X0, executed_before as u64);
    guards.push(asm.current_offset());
    asm.b(0);
    let resume = asm.current_offset();
    asm.patch_cbz_64(skip, X5, resume as i32 - skip as i32);
}

#[repr(C)]
struct BlockContext {
    memory_ptr: *mut GuestMemory,
    /// Set to 1 by a memory helper when a guest access faults. Translated code
    /// tests this word before committing any further architectural effect.
    faulted: u64,
    exit_reason: Option<ExitReason>,
}

const BLOCK_CTX_FAULTED_OFFSET: u32 = 8;
const _: () = assert!(std::mem::offset_of!(BlockContext, faulted) == 8);

extern "C" fn write_u64_helper(ctx: *mut BlockContext, addr: u64, val: u64) {
    unsafe {
        if (*ctx).exit_reason.is_some() {
            return;
        }
        let mem = &mut *(*ctx).memory_ptr;
        if let Err(e) = mem.write_u64(addr, val) {
            (*ctx).faulted = 1;
            (*ctx).exit_reason = Some(ExitReason::MemoryFault(e));
        }
    }
}

extern "C" fn read_u64_helper(ctx: *mut BlockContext, addr: u64) -> u64 {
    unsafe {
        if (*ctx).exit_reason.is_some() {
            return 0;
        }
        let mem = &*(*ctx).memory_ptr;
        match mem.read_u64(addr) {
            Ok(v) => v,
            Err(e) => {
                (*ctx).faulted = 1;
                (*ctx).exit_reason = Some(ExitReason::MemoryFault(e));
                0
            }
        }
    }
}

extern "C" fn set_exit_reason_helper(ctx: *mut BlockContext, code: u64) {
    unsafe {
        if code == 1 {
            (*ctx).exit_reason.get_or_insert(ExitReason::Halt);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::{MemoryError, MemoryPermissions};

    const ENTRY: u64 = 0x1000;

    fn fixture(code: &[u8]) -> (DbtEngine, CpuState, GuestMemory) {
        let mut memory = GuestMemory::new();
        memory
            .map_region_with_data(ENTRY, code.len(), MemoryPermissions::READ_EXECUTE, code)
            .unwrap();
        (
            DbtEngine::new(64 * 1024).unwrap(),
            CpuState::new(ENTRY, 0),
            memory,
        )
    }

    #[test]
    fn cached_block_obeys_remaining_budget() {
        let code = [
            0x48, 0x83, 0xC0, 0x80, // add rax, -128
            0x48, 0x83, 0xE8, 0xFF, // sub rax, -1
            0x48, 0x83, 0xE8, 0x02, // sub rax, 2
            0xF4, // hlt
        ];
        let (mut engine, initial, mut memory) = fixture(&code);

        let whole = engine.run(&mut initial.clone(), &mut memory, 16);
        assert_eq!(whole.instructions_executed, 4);
        assert_eq!(whole.exit_reason, ExitReason::Halt);
        assert_eq!(whole.final_state.get(Register::Rax), (-129_i64) as u64);

        for _ in 0..2 {
            let mut cpu = initial.clone();
            let limited = engine.run(&mut cpu, &mut memory, 2);
            assert_eq!(limited.instructions_executed, 2);
            assert_eq!(limited.exit_reason, ExitReason::StepLimitExceeded);
            assert_eq!(cpu.rip, ENTRY + 8);
            assert_eq!(cpu.get(Register::Rax), (-127_i64) as u64);

            let resumed = engine.run(&mut cpu, &mut memory, 2);
            assert_eq!(resumed.instructions_executed, 2);
            assert_eq!(resumed.exit_reason, ExitReason::Halt);
            assert_eq!(resumed.final_state, whole.final_state);
        }
        assert_eq!(engine.metrics.blocks_compiled, 3);
        assert_eq!(engine.metrics.cache_misses, 3);
        assert_eq!(engine.metrics.cache_hits, 2);
    }

    #[test]
    fn budget_does_not_decode_a_later_unsupported_opcode() {
        let code = [
            0x48, 0x83, 0xC0, 0x01, // add rax, 1
            0xCC, // int3 (unsupported)
        ];
        let (mut engine, mut cpu, mut memory) = fixture(&code);
        let limited = engine.run(&mut cpu, &mut memory, 1);
        assert_eq!(limited.exit_reason, ExitReason::StepLimitExceeded);
        assert_eq!(limited.instructions_executed, 1);
        assert_eq!(cpu.rip, ENTRY + 4);
        assert_eq!(cpu.get(Register::Rax), 1);
        let unsupported = engine.run(&mut cpu, &mut memory, 1);
        assert_eq!(
            unsupported.exit_reason,
            ExitReason::IllegalInstruction(0xCC)
        );
        assert_eq!(unsupported.instructions_executed, 0);
    }

    #[test]
    fn hlt_preserves_first_memory_fault() {
        let code = [
            0x48, 0x8B, 0x01, // mov rax, [rcx]
            0x48, 0x8B, 0x02, // mov rax, [rdx]
            0xF4, // hlt
        ];
        let (mut engine, mut cpu, mut memory) = fixture(&code);
        cpu.set(Register::Rcx, 0x2000);
        cpu.set(Register::Rdx, 0x3000);
        let result = engine.run(&mut cpu, &mut memory, 16);
        assert_eq!(
            result.exit_reason,
            ExitReason::MemoryFault(MemoryError::UnmappedAddress(0x2000))
        );
    }

    #[test]
    fn no_base_sib_does_not_emit_a_host_state_read() {
        let code = [
            0x48, 0x8B, 0x04, 0x25, 0, 0, 0, 0,    // mov rax, [0]
            0xF4, // hlt
        ];
        let (mut engine, mut cpu, mut memory) = fixture(&code);
        let result = engine.run(&mut cpu, &mut memory, 16);
        assert_eq!(result.exit_reason, ExitReason::IllegalInstruction(0x8B));
        assert_eq!(engine.metrics.blocks_compiled, 0);
    }

    #[test]
    fn indirect_call_target_fault_leaves_guest_stack_unchanged() {
        // call qword ptr [rip + 0x1000]; hlt. The IAT slot is outside the
        // mapped code region, so the read must fault before the push.
        let code = [0xFF, 0x15, 0x00, 0x10, 0x00, 0x00, 0xF4];
        let (mut engine, mut cpu, mut memory) = fixture(&code);
        memory
            .map_region(0x3000, 0x100, MemoryPermissions::READ_WRITE)
            .unwrap();
        cpu.set(Register::Rsp, 0x3080);
        let result = engine.run(&mut cpu, &mut memory, 16);
        assert_eq!(
            result.exit_reason,
            ExitReason::MemoryFault(MemoryError::UnmappedAddress(ENTRY + 0x1006))
        );
        assert_eq!(cpu.get(Register::Rsp), 0x3080);
    }

    #[test]
    fn indirect_register_call_preserves_stack_and_target() {
        // mov rax, 0x2000; call rax; hlt. 0x2000 is unmapped in this fixture,
        // so the translated call must fault after pushing and leave RIP at the
        // trampoline rather than executing translated bytes at an unknown address.
        let code = [
            0x48, 0xB8, 0x00, 0x20, 0, 0, 0, 0, 0, 0, // mov rax, 0x2000
            0xFF, 0xD0, // call rax
            0xF4, // hlt
        ];
        let (mut engine, mut cpu, mut memory) = fixture(&code);
        memory
            .map_region(0x3000, 0x100, MemoryPermissions::READ_WRITE)
            .unwrap();
        cpu.set(Register::Rsp, 0x3080);
        let result = engine.run(&mut cpu, &mut memory, 16);
        // The call itself succeeds; the next dispatch faults on unmapped code.
        assert!(
            matches!(result.exit_reason, ExitReason::MemoryFault(_)),
            "unexpected exit: {:?}",
            result.exit_reason
        );
        assert_eq!(cpu.get(Register::Rsp), 0x3078);
        assert_eq!(memory.read_u64(0x3078).unwrap(), ENTRY + 12);
    }

    #[test]
    fn exhausted_code_cache_reports_a_distinct_exit_reason() {
        const BLOCKS: usize = 128;
        const ADDS: usize = 16;
        let block_len = ADDS * 4 + 5;
        let mut code = vec![0u8; BLOCKS * block_len];
        for block in 0..BLOCKS {
            let start = block * block_len;
            for add in 0..ADDS {
                let at = start + add * 4;
                code[at..at + 4].copy_from_slice(&[0x48, 0x83, 0xC0, 0x01]);
            }
            let at = start + ADDS * 4;
            if block + 1 == BLOCKS {
                code[at] = 0xF4;
            } else {
                let next = (block + 1) * block_len;
                let rel = next as i32 - (at + 5) as i32;
                code[at] = 0xE9;
                code[at + 1..at + 5].copy_from_slice(&rel.to_le_bytes());
            }
        }
        let mut memory = GuestMemory::new();
        memory
            .map_region_with_data(ENTRY, code.len(), MemoryPermissions::READ_EXECUTE, &code)
            .unwrap();
        let mut engine = DbtEngine::new(16 * 1024).unwrap();
        let mut cpu = CpuState::new(ENTRY, 0);
        let result = engine.run(&mut cpu, &mut memory, BLOCKS * ADDS * 2);
        assert_eq!(result.exit_reason, ExitReason::JitCacheExhausted);
        assert!(engine.metrics.blocks_compiled < BLOCKS);
    }

    #[test]
    fn translated_blocks_balance_a_48_byte_frame() {
        let code = [0x48, 0x83, 0xC0, 0x01, 0xF4]; // add rax, 1; hlt
        let (mut engine, mut cpu, mut memory) = fixture(&code);
        let result = engine.run(&mut cpu, &mut memory, 16);
        assert_eq!(result.exit_reason, ExitReason::Halt);
        let emitted = engine.assembler.instructions();
        assert_eq!(emitted[0], 0xa9bd7bfd); // stp x29, x30, [sp, #-48]!
        assert_eq!(emitted[emitted.len() - 2], 0xa8c37bfd); // ldp x29, x30, [sp], #48
        assert_eq!(*emitted.last().unwrap(), 0xd65f03c0); // ret
    }

    #[test]
    fn faulting_push_leaves_rsp_and_guest_memory_unchanged() {
        // push rax with RSP aimed at an unmapped page. x86 raises the fault
        // without decrementing RSP or writing the stack.
        let code = [0x50, 0xF4]; // push rax; hlt
        let (mut engine, mut cpu, mut memory) = fixture(&code);
        cpu.set(Register::Rax, 0x1234_5678);
        cpu.set(Register::Rsp, 0x9000);
        let result = engine.run(&mut cpu, &mut memory, 16);
        assert_eq!(
            result.exit_reason,
            ExitReason::MemoryFault(MemoryError::UnmappedAddress(0x8FF8))
        );
        assert_eq!(cpu.get(Register::Rsp), 0x9000);
        assert_eq!(cpu.rip, ENTRY);
        assert_eq!(result.instructions_executed, 0);
    }

    #[test]
    fn mid_block_fault_reports_only_completed_instructions() {
        let code = [
            0x48, 0x83, 0xC0, 0x01, // add rax, 1
            0x48, 0x8B, 0x01, // mov rax, [rcx] (unmapped)
            0x48, 0x83, 0xC0, 0x01, // add rax, 1 (must not run)
            0xF4, // hlt
        ];
        let (mut engine, mut cpu, mut memory) = fixture(&code);
        cpu.set(Register::Rcx, 0x9000);
        let result = engine.run(&mut cpu, &mut memory, 16);
        assert_eq!(
            result.exit_reason,
            ExitReason::MemoryFault(MemoryError::UnmappedAddress(0x9000))
        );
        assert_eq!(result.instructions_executed, 1);
        assert_eq!(cpu.get(Register::Rax), 1);
        assert_eq!(cpu.rip, ENTRY + 4);
    }

    #[test]
    fn rip_relative_immediate_store_uses_end_of_instruction() {
        let code = [
            0x48, 0xC7, 0x05, 0xFA, 0x0F, 0, 0, 0xEF, 0xFF, 0xFF,
            0xFF, // mov [rip+0xffa], -17
            0xF4, // hlt
        ];
        let (mut engine, mut cpu, mut memory) = fixture(&code);
        memory
            .map_region(0x2000, 16, MemoryPermissions::READ_WRITE)
            .unwrap();
        let result = engine.run(&mut cpu, &mut memory, 2);
        assert_eq!(result.exit_reason, ExitReason::Halt);
        let mut stored = [0; 16];
        memory.read_bytes(0x2000, &mut stored).unwrap();
        assert_eq!(
            stored,
            [
                0, 0, 0, 0, 0, 0xEF, 255, 255, 255, 255, 255, 255, 255, 0, 0, 0
            ]
        );
    }
}
