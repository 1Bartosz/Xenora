// SPDX-License-Identifier: GPL-3.0-or-later

//! Bounded x86-64 to ARM64 Dynamic Binary Translation (DBT) engine for Xenora.
//!
//! Compiles decoded x86-64 basic blocks to native Apple Silicon ARM64 machine code,
//! executes them in a W^X code cache, and manages host-service dispatch and fallbacks.

use crate::arm64::{
    Arm64Assembler, Condition, SP, X0, X1, X2, X4, X5, X6, X9, X10, X11, X12, X13, X14, X15, X19,
    X20, X21, X29, X30, XZR,
};
use crate::cpu::{CpuState, ExecutionResult, ExitReason, RFlags, Register};
use crate::jit::{JitError, JitMemory};
use crate::memory::GuestMemory;
use crate::x86_decode::{Encoding, Insn, MemOperand, RegOperand, RmOperand, Segment, decode};
use crate::x86_exec;
use crate::x86_table::Op;
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
    /// Instructions run by the shared Rust semantics instead of ARM64 code.
    pub interpreter_fallbacks: usize,
    pub service_calls: usize,
}

/// Dynamic Binary Translator and native execution runtime.
pub struct DbtEngine {
    jit: JitMemory,
    block_cache: HashMap<u64, *const u8>,
    limited_block_cache: HashMap<(u64, usize), *const u8>,
    assembler: Arm64Assembler,
    /// Decoded instructions that emitted helper calls point into.
    decoded_blocks: Vec<Box<[Insn]>>,
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
            decoded_blocks: Vec::new(),
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
            let mut block = self.get_or_compile_block(cpu.rip, memory, block_limit);
            // A decode failure can belong to a later instruction in the block.
            // Execute the current instruction alone so earlier effects retire
            // and an earlier data fault takes precedence over speculative fetch.
            if block_limit > 1
                && matches!(
                    block,
                    Err(ExitReason::IllegalInstruction(_) | ExitReason::MemoryFault(_))
                )
            {
                block = self.get_or_compile_block(cpu.rip, memory, 1);
            }
            let block_fn_ptr = match block {
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
                helper_instructions: 0,
            };

            self.metrics.dbt_dispatches += 1;
            let cpu_raw_ptr = cpu as *mut CpuState as *mut u64;
            let insns_in_block = unsafe { block_fn(cpu_raw_ptr, &mut ctx) };
            instructions_count += insns_in_block as usize;
            self.metrics.instructions_executed += insns_in_block as usize;
            self.metrics.interpreter_fallbacks += ctx.helper_instructions as usize;

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
        // Decode the whole block first: lowering needs to know whether a later
        // instruction observes the flags an earlier one produces.
        let mut decoded = Vec::with_capacity(block_limit);
        let mut rip = start_rip;
        while decoded.len() < block_limit {
            let insn = decode(rip, |address| memory.fetch_instruction_byte(address))
                .map_err(ExitReason::from)?;
            if !is_translatable(&insn) {
                return Err(ExitReason::IllegalInstruction(0x05));
            }
            rip = insn.next_rip();
            decoded.push(insn);
            if insn.op.is_terminator() {
                break;
            }
        }
        // Emitted code holds pointers to these instructions for helper calls,
        // so the allocation lives as long as the engine's code cache.
        let insns = decoded.into_boxed_slice();

        // Reuse the emission buffer across cold block translations.
        let asm = &mut self.assembler;
        asm.clear();

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

        let mut emitter = Emitter {
            asm,
            fault_guards: Vec::new(),
        };
        for (index, insn) in insns.iter().enumerate() {
            let flags = flags_live_after(&insns, index);
            if is_native(insn) {
                emitter.lower(insn, index, flags);
            } else {
                emitter.call_semantics(insn, index);
            }
        }
        let Emitter { asm, fault_guards } = emitter;

        if !insns.last().is_some_and(|insn| insn.op.is_terminator()) {
            // End of basic block quota, update RIP.
            asm.load_imm_64(X9, rip);
            asm.str_uoff_64(X9, X19, RIP_OFFSET);
        }

        // Return number of guest instructions executed in this block in X0
        asm.load_imm_64(X0, insns.len() as u64);

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
        self.decoded_blocks.push(insns);
        Ok(emitted)
    }
}

const RIP_OFFSET: u32 = std::mem::offset_of!(CpuState, rip) as u32;
const FS_BASE_OFFSET: u32 = std::mem::offset_of!(CpuState, fs_base) as u32;
const GS_BASE_OFFSET: u32 = std::mem::offset_of!(CpuState, gs_base) as u32;
const FLAGS_OFFSET: u32 = std::mem::offset_of!(CpuState, flags) as u32;
const CF: u32 = FLAGS_OFFSET + std::mem::offset_of!(RFlags, cf) as u32;
const ZF: u32 = FLAGS_OFFSET + std::mem::offset_of!(RFlags, zf) as u32;
const SF: u32 = FLAGS_OFFSET + std::mem::offset_of!(RFlags, sf) as u32;
const OF: u32 = FLAGS_OFFSET + std::mem::offset_of!(RFlags, of) as u32;
const PF: u32 = FLAGS_OFFSET + std::mem::offset_of!(RFlags, pf) as u32;
const AF: u32 = FLAGS_OFFSET + std::mem::offset_of!(RFlags, af) as u32;
const ALL_FLAGS: u8 = 0x3F;

fn reg_offset(index: u8) -> u32 {
    u32::from(index) * 8
}

/// Whether the DBT runs an instruction at all. It dispatches guest services only
/// through registered imports, so a raw SYSCALL is refused.
pub fn is_translatable(insn: &Insn) -> bool {
    insn.op != Op::Syscall
}

/// Whether an instruction is lowered to ARM64. Every other decoded instruction
/// calls the shared semantics and is counted in
/// [`DbtMetrics::interpreter_fallbacks`].
pub fn is_native(insn: &Insn) -> bool {
    let wide = matches!(insn.size, 4 | 8);
    match insn.op {
        Op::Add | Op::Sub | Op::Cmp | Op::And | Op::Or | Op::Xor | Op::Test => wide,
        Op::Inc | Op::Dec | Op::Neg | Op::Not => wide,
        Op::Cmov => wide,
        Op::Pop => insn.encoding == Encoding::OpReg,
        Op::Mov
        | Op::Movzx
        | Op::Movsx
        | Op::Movsxd
        | Op::Lea
        | Op::Bswap
        | Op::Setcc
        | Op::Push
        | Op::Call
        | Op::Jmp
        | Op::Jcc
        | Op::Ret
        | Op::Nop
        | Op::Hlt => true,
        _ => false,
    }
}

/// Whether the flags produced by `insns[index]` can be observed. They cannot if
/// a later instruction in the block overwrites all of them before anything reads
/// them or could leave the block.
fn flags_live_after(insns: &[Insn], index: usize) -> bool {
    for insn in &insns[index + 1..] {
        let can_exit = insn.op.is_terminator()
            || insn.memory().is_some()
            || matches!(insn.op, Op::Push | Op::Pop | Op::Leave)
            || insn.op.can_fault();
        if can_exit || insn.op.flags_read() != 0 {
            return true;
        }
        if insn.op.flags_written() == ALL_FLAGS {
            return false;
        }
    }
    true
}

/// Emits one block. X19 holds the CPU state and X20 the block context; X21 is
/// the only guest value kept across helper calls.
struct Emitter<'a> {
    asm: &'a mut Arm64Assembler,
    fault_guards: Vec<usize>,
}

impl Emitter<'_> {
    fn call_helper(&mut self, helper: u64) {
        self.asm.mov_reg_64(X0, X20);
        self.asm.load_imm_64(X4, helper);
        self.asm.blr(X4);
    }

    fn guard(&mut self, insn: &Insn, completed: usize) {
        emit_fault_guard(self.asm, &mut self.fault_guards, completed, insn.rip);
    }

    /// Execute through the shared semantics.
    fn call_semantics(&mut self, insn: &Insn, completed: usize) {
        self.asm.mov_reg_64(X1, X19);
        self.asm
            .load_imm_64(X2, std::ptr::from_ref(insn) as usize as u64);
        self.call_helper(execute_helper as *const () as u64);
        self.guard(insn, completed);
    }

    /// Load a register operand into `xd`, with AH/CH/DH/BH in bits 7:0. Bits
    /// above the operand width are not cleared.
    fn load_reg(&mut self, xd: u8, reg: RegOperand) {
        self.asm.ldr_uoff_64(xd, X19, reg_offset(reg.index));
        if reg.high {
            self.asm.lsr_imm_64(xd, xd, 8);
        }
    }

    /// Store `xs` (not X13 or X14) with x86 partial-register rules.
    fn store_reg(&mut self, reg: RegOperand, size: u8, xs: u8) {
        let offset = reg_offset(reg.index);
        match size {
            8 => self.asm.str_uoff_64(xs, X19, offset),
            4 => {
                self.asm.mov_reg_32(X13, xs);
                self.asm.str_uoff_64(X13, X19, offset);
            }
            2 => {
                self.asm.uxth_32(X13, xs);
                self.asm.ldr_uoff_64(X14, X19, offset);
                self.asm.load_imm_64(X15, !0xFFFF);
                self.asm.and_reg_64(X14, X14, X15);
                self.asm.orr_reg_64(X13, X13, X14);
                self.asm.str_uoff_64(X13, X19, offset);
            }
            _ => self.asm.strb_uoff(xs, X19, offset + u32::from(reg.high)),
        }
    }

    /// Effective address into X1, clobbering X9.
    fn effective_address(&mut self, operand: &MemOperand, with_segment: bool) {
        let asm = &mut *self.asm;
        match operand.base {
            Some(base) => {
                asm.ldr_uoff_64(X1, X19, reg_offset(base));
                let disp = operand.disp;
                if (1..=4095).contains(&disp) {
                    asm.add_imm_64(X1, X1, disp as u16);
                } else if (-4095..=-1).contains(&disp) {
                    asm.sub_imm_64(X1, X1, -disp as u16);
                } else if disp != 0 {
                    asm.load_imm_64(X9, disp as u64);
                    asm.add_reg_64(X1, X1, X9);
                }
            }
            None => asm.load_imm_64(X1, operand.disp as u64),
        }
        if let Some((index, scale)) = operand.index {
            asm.ldr_uoff_64(X9, X19, reg_offset(index));
            if scale > 0 {
                asm.add_reg_shifted_64(X1, X1, X9, scale);
            } else {
                asm.add_reg_64(X1, X1, X9);
            }
        }
        if with_segment && let Some(segment) = operand.segment {
            let offset = match segment {
                Segment::Fs => FS_BASE_OFFSET,
                Segment::Gs => GS_BASE_OFFSET,
            };
            asm.ldr_uoff_64(X9, X19, offset);
            asm.add_reg_64(X1, X1, X9);
        }
    }

    /// Load the ModRM operand into `xd`. A register is loaded whole; a memory
    /// operand is zero-extended.
    fn load_rm(&mut self, xd: u8, insn: &Insn, completed: usize) {
        match insn.rm {
            Some(RmOperand::Reg(reg)) => self.load_reg(xd, reg),
            Some(RmOperand::Mem(operand)) => {
                self.effective_address(&operand, true);
                self.call_helper(load_helper(insn.rm_size));
                self.guard(insn, completed);
                self.asm.mov_reg_64(xd, X0);
            }
            None => {}
        }
    }

    /// Store `xs` to the ModRM operand. A memory store leaves no other effect
    /// behind if it faults, so callers commit registers and flags afterwards.
    fn store_rm(&mut self, insn: &Insn, xs: u8, completed: usize) {
        match insn.rm {
            Some(RmOperand::Reg(reg)) => self.store_reg(reg, insn.rm_size, xs),
            Some(RmOperand::Mem(operand)) => {
                self.asm.mov_reg_64(X2, xs);
                self.effective_address(&operand, true);
                self.call_helper(store_helper(insn.rm_size));
                self.guard(insn, completed);
            }
            None => {}
        }
    }

    /// X11 = X9 op X10, setting host NZCV only for the arithmetic operations.
    fn compute(&mut self, op: Op, wide: bool) {
        let asm = &mut *self.asm;
        match (op, wide) {
            (Op::Add | Op::Inc, true) => asm.adds_reg_64(X11, X9, X10),
            (Op::Add | Op::Inc, false) => asm.adds_reg_32(X11, X9, X10),
            (Op::And | Op::Test, true) => asm.ands_reg_64(X11, X9, X10),
            (Op::And | Op::Test, false) => asm.ands_reg_32(X11, X9, X10),
            (Op::Or, true) => asm.orr_reg_64(X11, X9, X10),
            (Op::Or, false) => asm.orr_reg_32(X11, X9, X10),
            (Op::Xor, _) => {
                asm.eor_reg_64(X11, X9, X10);
                if !wide {
                    asm.mov_reg_32(X11, X11);
                }
            }
            (_, true) => asm.subs_reg_64(X11, X9, X10),
            (_, false) => asm.subs_reg_32(X11, X9, X10),
        }
    }

    /// Store the guest flags of `X11 = X9 op X10` after [`Self::compute`].
    fn store_flags(&mut self, op: Op, wide: bool) {
        let asm = &mut *self.asm;
        let logical = matches!(op, Op::And | Op::Or | Op::Xor | Op::Test);
        if logical {
            if wide {
                asm.tst_reg_64(X11, X11);
            } else {
                asm.tst_reg_32(X11, X11);
            }
            asm.strb_uoff(XZR, X19, CF);
            asm.strb_uoff(XZR, X19, OF);
        } else {
            match op {
                // INC and DEC leave CF unchanged.
                Op::Inc | Op::Dec => {}
                Op::Add => {
                    asm.cset_32(X12, Condition::Cs);
                    asm.strb_uoff(X12, X19, CF);
                }
                // ARM C is set when a subtraction does not borrow; x86 CF is
                // the borrow itself.
                _ => {
                    asm.cset_32(X12, Condition::Cc);
                    asm.strb_uoff(X12, X19, CF);
                }
            }
            asm.cset_32(X12, Condition::Vs);
            asm.strb_uoff(X12, X19, OF);
        }
        asm.cset_32(X12, Condition::Eq);
        asm.strb_uoff(X12, X19, ZF);
        asm.cset_32(X12, Condition::Mi);
        asm.strb_uoff(X12, X19, SF);
        if !logical {
            // AF is the carry into bit 4: bit 4 of a ^ b ^ result.
            asm.eor_lsr_32(X12, X9, X10, 0);
            asm.eor_lsr_32(X12, X12, X11, 0);
            asm.ubfx_bit_32(X12, X12, 4);
            asm.strb_uoff(X12, X19, AF);
        }
        // PF is set when the low result byte has an even number of ones.
        asm.eor_lsr_32(X12, X11, X11, 4);
        asm.eor_lsr_32(X12, X12, X12, 2);
        asm.eor_lsr_32(X12, X12, X12, 1);
        asm.load_imm_64(X13, 1);
        asm.bic_reg_32(X12, X13, X12);
        asm.strb_uoff(X12, X19, PF);
    }

    /// W12 = 1 if the guest condition holds, else 0. Clobbers X13 and X14.
    fn load_condition(&mut self, code: u8) {
        let asm = &mut *self.asm;
        match code >> 1 {
            0 => asm.ldrb_uoff(X12, X19, OF),
            1 => asm.ldrb_uoff(X12, X19, CF),
            2 => asm.ldrb_uoff(X12, X19, ZF),
            3 => {
                asm.ldrb_uoff(X12, X19, CF);
                asm.ldrb_uoff(X13, X19, ZF);
                asm.orr_reg_32(X12, X12, X13);
            }
            4 => asm.ldrb_uoff(X12, X19, SF),
            5 => asm.ldrb_uoff(X12, X19, PF),
            other => {
                asm.ldrb_uoff(X12, X19, SF);
                asm.ldrb_uoff(X13, X19, OF);
                asm.eor_lsr_32(X12, X12, X13, 0);
                if other == 7 {
                    asm.ldrb_uoff(X13, X19, ZF);
                    asm.orr_reg_32(X12, X12, X13);
                }
            }
        }
        if code & 1 != 0 {
            asm.load_imm_64(X13, 1);
            asm.eor_lsr_32(X12, X12, X13, 0);
        }
    }

    /// Push `X2` on the guest stack, committing RSP only after the store.
    fn push(&mut self, insn: &Insn, completed: usize) {
        self.asm
            .ldr_uoff_64(X1, X19, reg_offset(Register::Rsp as u8));
        self.call_helper(push_helper as *const () as u64);
        self.guard(insn, completed);
        self.asm
            .str_uoff_64(X0, X19, reg_offset(Register::Rsp as u8));
    }

    /// Read the guest stack top into X0.
    fn read_stack(&mut self, insn: &Insn, completed: usize) {
        self.asm
            .ldr_uoff_64(X1, X19, reg_offset(Register::Rsp as u8));
        self.call_helper(load_helper(8));
        self.guard(insn, completed);
    }

    fn release_stack(&mut self, bytes: u64) {
        let offset = reg_offset(Register::Rsp as u8);
        self.asm.ldr_uoff_64(X10, X19, offset);
        self.asm.load_imm_64(X9, bytes);
        self.asm.add_reg_64(X10, X10, X9);
        self.asm.str_uoff_64(X10, X19, offset);
    }

    /// Branch target into `xd`.
    fn load_target(&mut self, xd: u8, insn: &Insn, completed: usize) {
        if insn.encoding == Encoding::Rel {
            self.asm.load_imm_64(xd, insn.imm);
        } else {
            self.load_rm(xd, insn, completed);
        }
    }

    /// Source operand of a two-operand ALU form into X10; memory sources of
    /// `reg, r/m` forms are loaded by the caller.
    fn load_source(&mut self, insn: &Insn) {
        match insn.encoding {
            Encoding::RmReg => self.load_reg(X10, insn.reg),
            _ => {
                let imm = match insn.op {
                    Op::Inc | Op::Dec => 1,
                    _ => insn.imm,
                };
                self.asm.load_imm_64(X10, imm);
            }
        }
    }

    fn alu(&mut self, insn: &Insn, completed: usize, flags: bool) {
        let wide = insn.size == 8;
        let writes = !matches!(insn.op, Op::Cmp | Op::Test);
        match insn.encoding {
            Encoding::RegRm => {
                self.load_rm(X10, insn, completed);
                self.load_reg(X9, insn.reg);
                self.compute(insn.op, wide);
                if flags {
                    self.store_flags(insn.op, wide);
                }
                if writes {
                    self.store_reg(insn.reg, insn.size, X11);
                }
            }
            Encoding::AccImm => {
                self.load_reg(X9, insn.reg);
                self.load_source(insn);
                self.compute(insn.op, wide);
                if flags {
                    self.store_flags(insn.op, wide);
                }
                if writes {
                    self.store_reg(insn.reg, insn.size, X11);
                }
            }
            _ => {
                self.load_rm(X9, insn, completed);
                self.load_source(insn);
                if insn.memory().is_some() && writes {
                    // Keep the old value across the store, then recompute the
                    // flags so a faulting store leaves them unchanged.
                    self.asm.mov_reg_64(X21, X9);
                    self.compute(insn.op, wide);
                    self.store_rm(insn, X11, completed);
                    if flags {
                        self.asm.mov_reg_64(X9, X21);
                        self.load_source(insn);
                        self.compute(insn.op, wide);
                        self.store_flags(insn.op, wide);
                    }
                    return;
                }
                self.compute(insn.op, wide);
                if flags {
                    self.store_flags(insn.op, wide);
                }
                if writes {
                    self.store_rm(insn, X11, completed);
                }
            }
        }
    }

    /// Lower an instruction accepted by [`is_native`].
    fn lower(&mut self, insn: &Insn, completed: usize, flags: bool) {
        match insn.op {
            Op::Nop => {}
            Op::Neg => {
                self.load_rm(X10, insn, completed);
                self.asm.mov_reg_64(X21, X10);
                self.asm.mov_reg_64(X9, XZR);
                self.compute(Op::Sub, insn.size == 8);
                self.store_rm(insn, X11, completed);
                if flags {
                    self.asm.mov_reg_64(X10, X21);
                    self.asm.mov_reg_64(X9, XZR);
                    self.compute(Op::Sub, insn.size == 8);
                    self.store_flags(Op::Sub, insn.size == 8);
                }
            }
            Op::Not => {
                self.load_rm(X9, insn, completed);
                self.asm.mvn_64(X11, X9);
                self.store_rm(insn, X11, completed);
            }
            Op::Mov => match insn.encoding {
                Encoding::RmReg => {
                    self.load_reg(X10, insn.reg);
                    self.store_rm(insn, X10, completed);
                }
                Encoding::RegRm => {
                    self.load_rm(X10, insn, completed);
                    self.store_reg(insn.reg, insn.size, X10);
                }
                Encoding::OpRegImm => {
                    self.asm.load_imm_64(X10, insn.imm);
                    self.store_reg(insn.reg, insn.size, X10);
                }
                _ => {
                    self.asm.load_imm_64(X10, insn.imm);
                    self.store_rm(insn, X10, completed);
                }
            },
            Op::Movzx | Op::Movsx | Op::Movsxd => {
                self.load_rm(X9, insn, completed);
                match (insn.op, insn.rm_size) {
                    (Op::Movzx, 1) => self.asm.uxtb_32(X10, X9),
                    (Op::Movzx, _) => self.asm.uxth_32(X10, X9),
                    (_, 1) => self.asm.sxtb_64(X10, X9),
                    (_, 2) => self.asm.sxth_64(X10, X9),
                    _ => self.asm.sxtw_64(X10, X9),
                }
                self.store_reg(insn.reg, insn.size, X10);
            }
            Op::Lea => {
                if let Some(operand) = insn.memory() {
                    self.effective_address(&operand, false);
                    self.store_reg(insn.reg, insn.size, X1);
                }
            }
            Op::Bswap => {
                self.load_reg(X9, insn.reg);
                self.asm.rev(X10, X9, insn.size == 8);
                self.store_reg(insn.reg, insn.size, X10);
            }
            Op::Setcc => {
                self.load_condition(insn.condition);
                self.store_rm(insn, X12, completed);
            }
            Op::Cmov => {
                self.load_rm(X10, insn, completed);
                self.load_reg(X9, insn.reg);
                self.load_condition(insn.condition);
                self.asm.tst_reg_32(X12, X12);
                self.asm.csel_64(X11, X10, X9, Condition::Ne);
                self.store_reg(insn.reg, insn.size, X11);
            }
            Op::Push => {
                match insn.encoding {
                    Encoding::OpReg => self.load_reg(X2, insn.reg),
                    Encoding::Imm => self.asm.load_imm_64(X2, insn.imm),
                    _ => self.load_rm(X2, insn, completed),
                }
                self.push(insn, completed);
            }
            Op::Pop => {
                self.read_stack(insn, completed);
                // POP RSP keeps the popped value, so it is written last.
                self.asm.mov_reg_64(X21, X0);
                self.release_stack(8);
                self.store_reg(insn.reg, 8, X21);
            }
            Op::Call => {
                // The target is read before the return address is pushed.
                self.load_target(X21, insn, completed);
                self.asm.load_imm_64(X2, insn.next_rip());
                self.push(insn, completed);
                self.asm.str_uoff_64(X21, X19, RIP_OFFSET);
            }
            Op::Jmp => {
                self.load_target(X9, insn, completed);
                self.asm.str_uoff_64(X9, X19, RIP_OFFSET);
            }
            Op::Jcc => {
                self.load_condition(insn.condition);
                self.asm.load_imm_64(X9, insn.next_rip());
                self.asm.load_imm_64(X10, insn.imm);
                self.asm.tst_reg_32(X12, X12);
                self.asm.csel_64(X9, X10, X9, Condition::Ne);
                self.asm.str_uoff_64(X9, X19, RIP_OFFSET);
            }
            Op::Ret => {
                self.read_stack(insn, completed);
                self.asm.str_uoff_64(X0, X19, RIP_OFFSET);
                self.release_stack(8u64.wrapping_add(insn.imm));
            }
            Op::Hlt => {
                self.asm.load_imm_64(X9, insn.next_rip());
                self.asm.str_uoff_64(X9, X19, RIP_OFFSET);
                self.asm.load_imm_64(X1, 1); // 1 = Halt
                self.call_helper(set_exit_reason_helper as *const () as u64);
            }
            _ => self.alu(insn, completed, flags),
        }
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
    /// Instructions executed through [`execute_helper`].
    helper_instructions: u64,
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

/// Host address of the store helper for an operand of `size` bytes.
fn store_helper(size: u8) -> u64 {
    let helper = match size {
        1 => write_u8_helper as *const (),
        2 => write_u16_helper as *const (),
        4 => write_u32_helper as *const (),
        _ => write_u64_helper as *const (),
    };
    helper as u64
}

/// Host address of the zero-extending load helper for `size` bytes.
fn load_helper(size: u8) -> u64 {
    let helper = match size {
        1 => read_u8_helper as *const (),
        2 => read_u16_helper as *const (),
        4 => read_u32_helper as *const (),
        _ => read_u64_helper as *const (),
    };
    helper as u64
}

/// Execute one decoded instruction through the shared semantics.
extern "C" fn execute_helper(ctx: *mut BlockContext, cpu: *mut CpuState, insn: *const Insn) {
    unsafe {
        (*ctx).helper_instructions += 1;
        match x86_exec::execute(&mut *cpu, &mut *(*ctx).memory_ptr, &*insn) {
            Ok(None) => {}
            Ok(Some(reason)) => (*ctx).exit_reason = Some(reason),
            Err(reason) => {
                (*ctx).faulted = 1;
                (*ctx).exit_reason = Some(reason);
            }
        }
    }
}

/// Push `value` below `sp` and return the new stack pointer.
extern "C" fn push_helper(ctx: *mut BlockContext, sp: u64, value: u64) -> u64 {
    unsafe {
        let Some(address) = sp.checked_sub(8) else {
            (*ctx).faulted = 1;
            (*ctx).exit_reason = Some(ExitReason::StackUnderflow);
            return sp;
        };
        write_u64_helper(ctx, address, value);
        address
    }
}

extern "C" fn write_u16_helper(ctx: *mut BlockContext, addr: u64, val: u64) {
    unsafe {
        if (*ctx).exit_reason.is_some() {
            return;
        }
        let mem = &mut *(*ctx).memory_ptr;
        if let Err(e) = mem.write_u16(addr, val as u16) {
            (*ctx).faulted = 1;
            (*ctx).exit_reason = Some(ExitReason::MemoryFault(e));
        }
    }
}

extern "C" fn write_u32_helper(ctx: *mut BlockContext, addr: u64, val: u64) {
    unsafe {
        if (*ctx).exit_reason.is_some() {
            return;
        }
        let mem = &mut *(*ctx).memory_ptr;
        if let Err(e) = mem.write_u32(addr, val as u32) {
            (*ctx).faulted = 1;
            (*ctx).exit_reason = Some(ExitReason::MemoryFault(e));
        }
    }
}

extern "C" fn read_u32_helper(ctx: *mut BlockContext, addr: u64) -> u64 {
    unsafe {
        if (*ctx).exit_reason.is_some() {
            return 0;
        }
        let mem = &*(*ctx).memory_ptr;
        match mem.read_u32(addr) {
            Ok(v) => u64::from(v),
            Err(e) => {
                (*ctx).faulted = 1;
                (*ctx).exit_reason = Some(ExitReason::MemoryFault(e));
                0
            }
        }
    }
}

extern "C" fn write_u8_helper(ctx: *mut BlockContext, addr: u64, val: u64) {
    unsafe {
        if (*ctx).exit_reason.is_some() {
            return;
        }
        let mem = &mut *(*ctx).memory_ptr;
        if let Err(e) = mem.write_u8(addr, val as u8) {
            (*ctx).faulted = 1;
            (*ctx).exit_reason = Some(ExitReason::MemoryFault(e));
        }
    }
}

extern "C" fn read_u8_helper(ctx: *mut BlockContext, addr: u64) -> u64 {
    unsafe {
        if (*ctx).exit_reason.is_some() {
            return 0;
        }
        let mem = &*(*ctx).memory_ptr;
        match mem.read_u8(addr) {
            Ok(v) => v as u64,
            Err(e) => {
                (*ctx).faulted = 1;
                (*ctx).exit_reason = Some(ExitReason::MemoryFault(e));
                0
            }
        }
    }
}

extern "C" fn read_u16_helper(ctx: *mut BlockContext, addr: u64) -> u64 {
    unsafe {
        if (*ctx).exit_reason.is_some() {
            return 0;
        }
        let mem = &*(*ctx).memory_ptr;
        match mem.read_u16(addr) {
            Ok(v) => v as u64,
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
    fn completed_store_survives_the_next_fault_or_budget_boundary() {
        for native in [false, true] {
            for (tail, fault) in [
                (&[0xcc][..], ExitReason::IllegalInstruction(0xcc)),
                (
                    &[0xb8, 1][..],
                    ExitReason::MemoryFault(MemoryError::UnmappedAddress(ENTRY + 5)),
                ),
                (
                    &[0x48, 0x8b, 0x02, 0xf4][..],
                    ExitReason::MemoryFault(MemoryError::UnmappedAddress(0x4000)),
                ),
            ] {
                for budget in [0, 1, 16] {
                    // mov [rcx], rax; then an illegal, truncated or faulting instruction.
                    let code = [&[0x48, 0x89, 0x01][..], tail].concat();
                    let (mut engine, mut cpu, mut memory) = fixture(&code);
                    memory
                        .map_region(0x3000, 8, MemoryPermissions::READ_WRITE)
                        .unwrap();
                    cpu.set(Register::Rax, 0x1234);
                    cpu.set(Register::Rcx, 0x3000);
                    cpu.set(Register::Rdx, 0x4000);
                    let mut expected = cpu.clone();
                    if budget > 0 {
                        expected.rip += 3;
                    }
                    let result = if native {
                        engine.run(&mut cpu, &mut memory, budget)
                    } else {
                        crate::cpu::Interpreter::new(cpu, &mut memory, budget).run()
                    };
                    assert_eq!(result.instructions_executed, budget.min(1));
                    assert_eq!(
                        result.exit_reason,
                        if budget <= 1 {
                            ExitReason::StepLimitExceeded
                        } else {
                            fault
                        }
                    );
                    assert_eq!(result.final_state, expected);
                    assert_eq!(
                        memory.read_u64(0x3000).unwrap(),
                        if budget == 0 { 0 } else { 0x1234 }
                    );
                }
            }
        }
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
    fn no_base_sib_computes_displacement_without_base() {
        // mov rax, [0x2000]; hlt
        let code = [
            0x48, 0x8B, 0x04, 0x25, 0x00, 0x20, 0, 0,    // mov rax, [0x2000]
            0xF4, // hlt
        ];
        let (mut engine, mut cpu, mut memory) = fixture(&code);
        memory
            .map_region(0x2000, 16, MemoryPermissions::READ_WRITE)
            .unwrap();
        memory.write_u64(0x2000, 0x1122_3344_5566_7788).unwrap();
        let result = engine.run(&mut cpu, &mut memory, 16);
        assert_eq!(result.exit_reason, ExitReason::Halt);
        assert_eq!(cpu.get(Register::Rax), 0x1122_3344_5566_7788);
    }

    #[test]
    fn sib_store_executes_through_the_native_dbt() {
        // mov [rsp + 0x20], rbx; hlt. The SIB store form that the static
        // probe used to report as a startup blocker.
        let code = [
            0x48, 0x89, 0x5c, 0x24, 0x20, // mov [rsp + 0x20], rbx
            0xF4, // hlt
        ];
        let (mut engine, mut cpu, mut memory) = fixture(&code);
        const STACK: u64 = 0x4000;
        const STORED_ADDR: u64 = 0x4020;
        const STORED_VALUE: u64 = 0x1122_3344_5566_7788;
        cpu.set(Register::Rsp, STACK);
        cpu.set(Register::Rbx, STORED_VALUE);
        memory
            .map_region(STACK, 0x100, MemoryPermissions::READ_WRITE)
            .unwrap();
        let result = engine.run(&mut cpu, &mut memory, 16);
        assert_eq!(result.exit_reason, ExitReason::Halt);
        assert_eq!(memory.read_u64(STORED_ADDR).unwrap(), STORED_VALUE);
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
