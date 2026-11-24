// SPDX-License-Identifier: GPL-3.0-or-later

//! End-to-end integration tests for Xenora X12: PE32+ loading & x86-64 execution bootstrap.

use xenora_loader::cpu::{CpuState, ExitReason, Interpreter, Register};
use xenora_loader::memory::{GuestMemory, MemoryError, MemoryPermissions};
use xenora_loader::pe::{
    IMAGE_FILE_MACHINE_AMD64, IMAGE_SCN_MEM_EXECUTE, IMAGE_SCN_MEM_READ, PE32_PLUS_MAGIC, ParsedPe,
};
use xenora_loader::triage::triage_bytes;

/// Helper to build a valid synthetic PE32+ executable with a single .text section.
fn build_synthetic_pe32_plus(code: &[u8]) -> Vec<u8> {
    let mut pe = Vec::new();

    // 1. DOS Header (64 bytes)
    pe.extend_from_slice(b"MZ");
    pe.resize(0x3C, 0);
    // e_lfanew at 0x3C points to 0x80
    let e_lfanew: u32 = 0x80;
    pe.extend_from_slice(&e_lfanew.to_le_bytes());
    pe.resize(0x80, 0);

    // 2. PE Signature
    pe.extend_from_slice(b"PE\0\0");

    // 3. COFF File Header (20 bytes)
    pe.extend_from_slice(&IMAGE_FILE_MACHINE_AMD64.to_le_bytes()); // Machine: 0x8664
    let num_sections: u16 = 1;
    pe.extend_from_slice(&num_sections.to_le_bytes());
    let time_stamp: u32 = 0x67300000;
    pe.extend_from_slice(&time_stamp.to_le_bytes());
    pe.extend_from_slice(&0u32.to_le_bytes()); // PointerToSymbolTable
    pe.extend_from_slice(&0u32.to_le_bytes()); // NumberOfSymbols
    let opt_hdr_size: u16 = 112; // Standard size without data directories
    pe.extend_from_slice(&opt_hdr_size.to_le_bytes());
    let characteristics: u16 = 0x0022; // EXECUTABLE_IMAGE | LARGE_ADDRESS_AWARE
    pe.extend_from_slice(&characteristics.to_le_bytes());

    // 4. Optional Header (PE32+) (112 bytes)
    pe.extend_from_slice(&PE32_PLUS_MAGIC.to_le_bytes()); // Magic: 0x020B
    pe.push(14); // MajorLinkerVersion
    pe.push(0); // MinorLinkerVersion
    pe.extend_from_slice(&(code.len() as u32).to_le_bytes()); // SizeOfCode
    pe.extend_from_slice(&0u32.to_le_bytes()); // SizeOfInitializedData
    pe.extend_from_slice(&0u32.to_le_bytes()); // SizeOfUninitializedData
    let entry_point_rva: u32 = 0x1000;
    pe.extend_from_slice(&entry_point_rva.to_le_bytes()); // AddressOfEntryPoint
    pe.extend_from_slice(&0x1000u32.to_le_bytes()); // BaseOfCode
    let image_base: u64 = 0x0000000140000000;
    pe.extend_from_slice(&image_base.to_le_bytes()); // ImageBase
    pe.extend_from_slice(&0x1000u32.to_le_bytes()); // SectionAlignment
    pe.extend_from_slice(&0x200u32.to_le_bytes()); // FileAlignment
    pe.extend_from_slice(&6u16.to_le_bytes()); // MajorOSVersion
    pe.extend_from_slice(&0u16.to_le_bytes()); // MinorOSVersion
    pe.extend_from_slice(&0u16.to_le_bytes()); // MajorImageVersion
    pe.extend_from_slice(&0u16.to_le_bytes()); // MinorImageVersion
    pe.extend_from_slice(&6u16.to_le_bytes()); // MajorSubsystemVersion
    pe.extend_from_slice(&0u16.to_le_bytes()); // MinorSubsystemVersion
    pe.extend_from_slice(&0u32.to_le_bytes()); // Win32VersionValue
    pe.extend_from_slice(&0x3000u32.to_le_bytes()); // SizeOfImage
    let size_of_headers: u32 = 0x200;
    pe.extend_from_slice(&size_of_headers.to_le_bytes()); // SizeOfHeaders
    pe.extend_from_slice(&0u32.to_le_bytes()); // CheckSum
    pe.extend_from_slice(&3u16.to_le_bytes()); // Subsystem: WINDOWS_CUI
    pe.extend_from_slice(&0x8160u16.to_le_bytes()); // DllCharacteristics
    pe.extend_from_slice(&0x100000u64.to_le_bytes()); // SizeOfStackReserve
    pe.extend_from_slice(&0x1000u64.to_le_bytes()); // SizeOfStackCommit
    pe.extend_from_slice(&0x100000u64.to_le_bytes()); // SizeOfHeapReserve
    pe.extend_from_slice(&0x1000u64.to_le_bytes()); // SizeOfHeapCommit
    pe.extend_from_slice(&0u32.to_le_bytes()); // LoaderFlags
    pe.extend_from_slice(&0u32.to_le_bytes()); // NumberOfRvaAndSizes

    // 5. Section Table (1 section: .text, 40 bytes)
    let mut sec_name = [0u8; 8];
    sec_name[..5].copy_from_slice(b".text");
    pe.extend_from_slice(&sec_name);
    let virtual_size: u32 = code.len() as u32;
    pe.extend_from_slice(&virtual_size.to_le_bytes()); // VirtualSize
    pe.extend_from_slice(&0x1000u32.to_le_bytes()); // VirtualAddress
    let raw_size: u32 = 0x200;
    pe.extend_from_slice(&raw_size.to_le_bytes()); // SizeOfRawData
    let raw_ptr: u32 = 0x200;
    pe.extend_from_slice(&raw_ptr.to_le_bytes()); // PointerToRawData
    pe.extend_from_slice(&0u32.to_le_bytes()); // PointerToRelocations
    pe.extend_from_slice(&0u32.to_le_bytes()); // PointerToLinenumbers
    pe.extend_from_slice(&0u16.to_le_bytes()); // NumberOfRelocations
    pe.extend_from_slice(&0u16.to_le_bytes()); // NumberOfLinenumbers
    let characteristics: u32 = IMAGE_SCN_MEM_READ | IMAGE_SCN_MEM_EXECUTE; // 0x60000020
    pe.extend_from_slice(&characteristics.to_le_bytes());

    // Pad headers to FileAlignment (0x200)
    pe.resize(0x200, 0);

    // Section raw data (.text)
    pe.extend_from_slice(code);
    pe.resize(0x400, 0); // Pad section to 0x200 raw size

    pe
}

#[test]
fn test_pe_triage_and_parsing() {
    let dummy_code = [0x90, 0xF4]; // NOP; HLT
    let pe_bytes = build_synthetic_pe32_plus(&dummy_code);

    let triage = triage_bytes(&pe_bytes).expect("triage must succeed");
    assert!(triage.is_amd64);
    assert_eq!(triage.image_base, 0x140000000);
    assert_eq!(triage.entry_point_rva, 0x1000);
    assert_eq!(triage.sections.len(), 1);
    assert_eq!(triage.sections[0].name, ".text");
    assert!(triage.sections[0].permissions.execute);
    assert!(triage.sections[0].permissions.read);
    assert!(!triage.sections[0].permissions.write);
}

#[test]
fn test_pe_loading_and_execution() {
    // x86-64 program:
    // mov rax, 42         -> 48 b8 2a 00 00 00 00 00 00 00
    // mov rcx, 10         -> 48 b9 0a 00 00 00 00 00 00 00
    // add rax, rcx        -> 48 01 c8
    // hlt                 -> f4
    let mut code = Vec::new();
    // MOV RAX, 42
    code.extend_from_slice(&[0x48, 0xB8, 42, 0, 0, 0, 0, 0, 0, 0]);
    // MOV RCX, 10
    code.extend_from_slice(&[0x48, 0xB9, 10, 0, 0, 0, 0, 0, 0, 0]);
    // ADD RAX, RCX
    code.extend_from_slice(&[0x48, 0x01, 0xC8]);
    // HLT
    code.push(0xF4);

    let pe_bytes = build_synthetic_pe32_plus(&code);
    let parsed = ParsedPe::parse(&pe_bytes).expect("PE parse should succeed");
    let mut loaded = parsed
        .load_into_memory(&pe_bytes)
        .expect("PE load should succeed");

    // Set up a guest stack region at 0x20000000 (RW)
    let stack_base = 0x2000_0000u64;
    let stack_size = 0x10000usize;
    loaded
        .memory
        .map_region(stack_base, stack_size, MemoryPermissions::READ_WRITE)
        .expect("stack mapping should succeed");
    let stack_top = stack_base + stack_size as u64;

    let cpu = CpuState::new(loaded.entry_point, stack_top);
    let mut interp = Interpreter::new(cpu, &mut loaded.memory, 1000);
    let result = interp.run();

    assert_eq!(result.exit_reason, ExitReason::Halt);
    assert_eq!(result.instructions_executed, 4);
    assert_eq!(result.final_state.get(Register::Rax), 52);
    assert_eq!(result.final_state.get(Register::Rcx), 10);
    assert!(!result.final_state.flags.zf);
}

#[test]
fn test_stack_operations_and_subroutine_call() {
    // x86-64 program:
    // mov rdx, 0x1234      (48 ba 34 12 00 00 00 00 00 00)
    // push rdx             (52)
    // pop rbx              (5b)
    // call func            (e8 05 00 00 00)
    // hlt                  (f4)
    // func:
    // add rbx, 1           (48 83 c3 01)
    // ret                  (c3)
    let mut code = Vec::new();
    code.extend_from_slice(&[0x48, 0xBA, 0x34, 0x12, 0, 0, 0, 0, 0, 0]); // mov rdx, 0x1234
    code.push(0x52); // push rdx
    code.push(0x5B); // pop rbx
    code.extend_from_slice(&[0xE8, 0x01, 0x00, 0x00, 0x00]); // call +1
    code.push(0xF4); // hlt
    code.extend_from_slice(&[0x48, 0x83, 0xC3, 0x01]); // add rbx, 1
    code.push(0xC3); // ret

    let pe_bytes = build_synthetic_pe32_plus(&code);
    let parsed = ParsedPe::parse(&pe_bytes).expect("PE parse should succeed");
    let mut loaded = parsed
        .load_into_memory(&pe_bytes)
        .expect("PE load should succeed");

    let stack_base = 0x3000_0000u64;
    let stack_size = 0x10000usize;
    loaded
        .memory
        .map_region(stack_base, stack_size, MemoryPermissions::READ_WRITE)
        .expect("stack mapping should succeed");
    let stack_top = stack_base + stack_size as u64;

    let cpu = CpuState::new(loaded.entry_point, stack_top);
    let mut interp = Interpreter::new(cpu, &mut loaded.memory, 1000);
    let result = interp.run();

    assert_eq!(result.exit_reason, ExitReason::Halt);
    assert_eq!(result.final_state.get(Register::Rbx), 0x1235);
    assert_eq!(result.final_state.get(Register::Rsp), stack_top);
}

#[test]
fn test_step_budget_exhaustion() {
    // Infinite loop: jmp -2 (EB FE)
    let code = [0xEB, 0xFE];
    let pe_bytes = build_synthetic_pe32_plus(&code);
    let parsed = ParsedPe::parse(&pe_bytes).expect("parse");
    let mut loaded = parsed.load_into_memory(&pe_bytes).expect("load");

    let cpu = CpuState::new(loaded.entry_point, 0x10000);
    let mut interp = Interpreter::new(cpu, &mut loaded.memory, 50);
    let result = interp.run();

    assert_eq!(result.exit_reason, ExitReason::StepLimitExceeded);
    assert_eq!(result.instructions_executed, 50);
}

#[test]
fn test_unsupported_instruction_fails_deterministically() {
    // Unsupported opcode: 0x0F 0x0B (UD2)
    let code = [0x0F, 0x0B];
    let pe_bytes = build_synthetic_pe32_plus(&code);
    let parsed = ParsedPe::parse(&pe_bytes).expect("parse");
    let mut loaded = parsed.load_into_memory(&pe_bytes).expect("load");

    let cpu = CpuState::new(loaded.entry_point, 0x10000);
    let mut interp = Interpreter::new(cpu, &mut loaded.memory, 50);
    let result = interp.run();

    assert_eq!(result.exit_reason, ExitReason::IllegalInstruction(0x0B));
}

#[test]
fn test_memory_fault_on_unmapped_access() {
    // MOV RAX, [0xDEADBEEF] -> 48 8B 04 25 EF BE AD DE (or simple reg-disp mov)
    // Here: mov rax, 0xDEADBEEF; mov rbx, [rax]
    // 48 b8 ef be ad de 00 00 00 00
    // 48 8b 18 (mov rbx, [rax])
    let code = [
        0x48, 0xB8, 0xEF, 0xBE, 0xAD, 0xDE, 0x00, 0x00, 0x00, 0x00, 0x48, 0x8B, 0x18,
    ];
    let pe_bytes = build_synthetic_pe32_plus(&code);
    let parsed = ParsedPe::parse(&pe_bytes).expect("parse");
    let mut loaded = parsed.load_into_memory(&pe_bytes).expect("load");

    let cpu = CpuState::new(loaded.entry_point, 0x10000);
    let mut interp = Interpreter::new(cpu, &mut loaded.memory, 50);
    let result = interp.run();

    match result.exit_reason {
        ExitReason::MemoryFault(MemoryError::UnmappedAddress(addr)) => {
            assert_eq!(addr, 0xDEADBEEF);
        }
        other => panic!("expected UnmappedAddress, got: {other:?}"),
    }
}

#[test]
fn test_sib_addressing_mode() {
    // x86-64 program:
    // mov rbx, 0x12345678_9abcdef0
    // mov [rsp + 0x20], rbx   (48 89 5c 24 20 - using SIB byte 0x24)
    // mov rdx, [rsp + 0x20]   (48 8b 54 24 20 - using SIB byte 0x24)
    // hlt                     (f4)
    let mut code = Vec::new();
    code.extend_from_slice(&[0x48, 0xBB, 0xF0, 0xDE, 0xBC, 0x9A, 0x78, 0x56, 0x34, 0x12]);
    code.extend_from_slice(&[0x48, 0x89, 0x5C, 0x24, 0x20]); // mov [rsp + 0x20], rbx
    code.extend_from_slice(&[0x48, 0x8B, 0x54, 0x24, 0x20]); // mov rdx, [rsp + 0x20]
    code.push(0xF4); // hlt

    let pe_bytes = build_synthetic_pe32_plus(&code);
    let parsed = ParsedPe::parse(&pe_bytes).expect("parse");
    let mut loaded = parsed.load_into_memory(&pe_bytes).expect("load");

    let stack_base = 0x4000_0000u64;
    let stack_size = 0x10000usize;
    loaded
        .memory
        .map_region(stack_base, stack_size, MemoryPermissions::READ_WRITE)
        .expect("stack mapping should succeed");
    let stack_top = stack_base + 0x8000;

    let cpu = CpuState::new(loaded.entry_point, stack_top);
    let mut interp = Interpreter::new(cpu, &mut loaded.memory, 100);
    let result = interp.run();

    assert_eq!(result.exit_reason, ExitReason::Halt);
    assert_eq!(result.final_state.get(Register::Rdx), 0x12345678_9abcdef0);
}

/// Base address of the read-write data region used by the SIB tests.
const DATA_BASE: u64 = 0x6000_0000;
/// Base address of the read-write stack region used by the SIB tests.
const STACK_BASE: u64 = 0x5000_0000;

/// Run `code` on the interpreter with a writable stack and a writable data
/// region, and return the final state, the exit reason and the guest memory.
///
/// `DATA_BASE` is mapped read-write so a test can observe where a store
/// landed. `STACK_BASE` holds the stack pointer.
fn run_with_regions(code: &[u8]) -> (CpuState, ExitReason, GuestMemory) {
    let pe_bytes = build_synthetic_pe32_plus(code);
    let parsed = ParsedPe::parse(&pe_bytes).expect("parse");
    let mut loaded = parsed.load_into_memory(&pe_bytes).expect("load");

    loaded
        .memory
        .map_region(STACK_BASE, 0x10000, MemoryPermissions::READ_WRITE)
        .expect("stack mapping should succeed");
    loaded
        .memory
        .map_region(DATA_BASE, 0x10000, MemoryPermissions::READ_WRITE)
        .expect("data mapping should succeed");

    let cpu = CpuState::new(loaded.entry_point, STACK_BASE + 0x8000);
    let mut interp = Interpreter::new(cpu, &mut loaded.memory, 100);
    let result = interp.run();
    (result.final_state, result.exit_reason, loaded.memory)
}

/// `mov rax, DATA_BASE` (48 B8 <imm64>).
fn mov_rax_data_base() -> Vec<u8> {
    let mut code = vec![0x48, 0xB8];
    code.extend_from_slice(&DATA_BASE.to_le_bytes());
    code
}

/// `mov rbx, 0x12345678_9abcdef0` (48 BB <imm64>).
fn mov_rbx_value() -> Vec<u8> {
    let mut code = vec![0x48, 0xBB];
    code.extend_from_slice(&0x1234_5678_9abc_def0u64.to_le_bytes());
    code
}

/// A SIB index register is extended by REX.X, not by REX.R.
///
/// The encoding 4A 89 1C E0 is `mov [rax + r12*8], rbx`. Its REX prefix sets
/// REX.X, so the SIB index field 100b selects r12 and the store lands at
/// `rax + r12*8`. REX.R is clear in this prefix, so the ModRM `reg` field is
/// rbx.
#[test]
fn test_rex_x_extends_the_sib_index_register() {
    // mov rax, DATA_BASE           (48 b8 <imm64>)
    // mov r12, 4                   (41 bc 04 00 00 00)
    // mov rbx, 0x12345678_9abcdef0 (48 bb <imm64>)
    // mov [rax + r12*8], rbx       (4a 89 1c e0)
    // hlt                          (f4)
    let mut code = mov_rax_data_base();
    code.extend_from_slice(&[0x41, 0xBC, 0x04, 0x00, 0x00, 0x00]); // mov r12d, 4
    code.extend_from_slice(&mov_rbx_value());
    code.extend_from_slice(&[0x4A, 0x89, 0x1C, 0xE0]); // mov [rax + r12*8], rbx
    code.push(0xF4); // hlt

    let (_state, reason, memory) = run_with_regions(&code);
    assert_eq!(reason, ExitReason::Halt);
    // r12 holds 4, so the store lands at DATA_BASE + 4*8.
    assert_eq!(
        memory.read_u64(DATA_BASE + 32).expect("indexed slot"),
        0x1234_5678_9abc_def0
    );
    // The unindexed slot at DATA_BASE must be untouched.
    assert_eq!(memory.read_u64(DATA_BASE).expect("base slot"), 0);
}

/// REX.R and REX.X extend independent fields in the same instruction.
///
/// The encoding 4C 89 1C C8 is `mov [rax + rcx*8], r11`. Its REX prefix sets
/// REX.R, so the ModRM `reg` field 011 selects r11. It does not set REX.X, so
/// the SIB index field 001 selects rcx and must not be extended to r9.
#[test]
fn test_rex_r_does_not_extend_the_sib_index_register() {
    // mov rax, DATA_BASE           (48 b8 <imm64>)
    // mov r11, 0x11223344_55667788 (49 bb <imm64>)
    // mov ecx, 4                   (b9 04 00 00 00)
    // mov r9, 8                    (41 b9 08 00 00 00)
    // mov [rax + rcx*8], r11       (4c 89 1c c8)
    // hlt                          (f4)
    let mut code = mov_rax_data_base();
    let mut r11 = vec![0x49, 0xBB];
    r11.extend_from_slice(&0x1122_3344_5566_7788u64.to_le_bytes());
    code.extend_from_slice(&r11);
    code.extend_from_slice(&[0xB9, 0x04, 0x00, 0x00, 0x00]); // mov ecx, 4
    code.extend_from_slice(&[0x41, 0xB9, 0x08, 0x00, 0x00, 0x00]); // mov r9d, 8
    code.extend_from_slice(&[0x4C, 0x89, 0x1C, 0xC8]); // mov [rax + rcx*8], r11
    code.push(0xF4); // hlt

    let (_state, reason, memory) = run_with_regions(&code);
    assert_eq!(reason, ExitReason::Halt);
    // rcx holds 4, so the store lands at DATA_BASE + 4*8. A wrongly extended
    // index would have selected r9 and landed at DATA_BASE + 8*8.
    assert_eq!(
        memory.read_u64(DATA_BASE + 32).expect("indexed slot"),
        0x1122_3344_5566_7788
    );
    assert_eq!(memory.read_u64(DATA_BASE + 64).expect("r9 slot"), 0);
}

/// REX.R and REX.X set together extend their own fields independently.
///
/// The encoding 4E 89 1C E0 is `mov [rax + r12*8], r11`. Its REX prefix sets
/// both REX.R and REX.X: the ModRM `reg` field 011 becomes r11 and the SIB
/// index field 100b becomes r12.
#[test]
fn test_rex_r_and_rex_x_set_together_extend_independent_fields() {
    // mov rax, DATA_BASE           (48 b8 <imm64>)
    // mov r11, 0x11223344_55667788 (49 bb <imm64>)
    // mov r12, 4                   (41 bc 04 00 00 00)
    // mov [rax + r12*8], r11       (4e 89 1c e0)
    // hlt                          (f4)
    let mut code = mov_rax_data_base();
    let mut r11 = vec![0x49, 0xBB];
    r11.extend_from_slice(&0x1122_3344_5566_7788u64.to_le_bytes());
    code.extend_from_slice(&r11);
    code.extend_from_slice(&[0x41, 0xBC, 0x04, 0x00, 0x00, 0x00]); // mov r12d, 4
    code.extend_from_slice(&[0x4E, 0x89, 0x1C, 0xE0]); // mov [rax + r12*8], r11
    code.push(0xF4); // hlt

    let (_state, reason, memory) = run_with_regions(&code);
    assert_eq!(reason, ExitReason::Halt);
    assert_eq!(
        memory.read_u64(DATA_BASE + 32).expect("indexed slot"),
        0x1122_3344_5566_7788
    );
    assert_eq!(memory.read_u64(DATA_BASE).expect("base slot"), 0);
}

/// Without REX.X, a SIB index field of 100b means "no index register" and the
/// address is the base alone.
///
/// The encoding 48 89 1C 24 is `mov [rsp], rbx`: the SIB index field is 100b
/// and REX.X is clear, so no index register contributes to the address.
#[test]
fn test_sib_index_field_100b_without_rex_x_means_no_index() {
    // mov rbx, 0x12345678_9abcdef0 (48 bb <imm64>)
    // mov r12, 0x1000              (41 bc 00 10 00 00)
    // mov [rsp], rbx              (48 89 1c 24)
    // hlt                          (f4)
    let mut code = mov_rbx_value();
    code.extend_from_slice(&[0x41, 0xBC, 0x00, 0x10, 0x00, 0x00]); // mov r12d, 0x1000
    code.extend_from_slice(&[0x48, 0x89, 0x1C, 0x24]); // mov [rsp], rbx
    code.push(0xF4); // hlt

    let pe_bytes = build_synthetic_pe32_plus(&code);
    let parsed = ParsedPe::parse(&pe_bytes).expect("parse");
    let mut loaded = parsed.load_into_memory(&pe_bytes).expect("load");
    loaded
        .memory
        .map_region(STACK_BASE, 0x10000, MemoryPermissions::READ_WRITE)
        .expect("stack mapping should succeed");

    let cpu = CpuState::new(loaded.entry_point, STACK_BASE + 0x8000);
    let mut interp = Interpreter::new(cpu, &mut loaded.memory, 100);
    let result = interp.run();
    assert_eq!(result.exit_reason, ExitReason::Halt);

    // The index is absent, so the address is the base (rsp) alone.
    let stack_top = STACK_BASE + 0x8000;
    assert_eq!(
        loaded.memory.read_u64(stack_top).expect("base slot"),
        0x1234_5678_9abc_def0
    );
}

/// Build a guest program that loads `value` into `rdi` and then runs `body`.
///
/// `mov rdi, imm64` is 48 BF <imm64>.
fn program_with_rdi(value: u64, body: &[u8]) -> Vec<u8> {
    let mut code = vec![0x48, 0xBF];
    code.extend_from_slice(&value.to_le_bytes());
    code.extend_from_slice(body);
    code.push(0xF4); // hlt
    code
}

/// Run `code` on the interpreter and return the final state and exit reason.
fn run_interpreter(code: &[u8]) -> (CpuState, ExitReason) {
    let pe_bytes = build_synthetic_pe32_plus(code);
    let parsed = ParsedPe::parse(&pe_bytes).expect("parse");
    let mut loaded = parsed.load_into_memory(&pe_bytes).expect("load");
    loaded
        .memory
        .map_region(STACK_BASE, 0x10000, MemoryPermissions::READ_WRITE)
        .expect("stack mapping should succeed");
    loaded
        .memory
        .map_region(DATA_BASE, 0x10000, MemoryPermissions::READ_WRITE)
        .expect("data mapping should succeed");

    let cpu = CpuState::new(loaded.entry_point, STACK_BASE + 0x8000);
    let mut interp = Interpreter::new(cpu, &mut loaded.memory, 100);
    let result = interp.run();
    (result.final_state, result.exit_reason)
}

/// MOVZX with a register source, for both destination widths.
///
/// The encodings are `movzbl eax, bl` (0F B6 C3) and
/// `movzbq rdx, bl` (48 0F B6 D3), as produced by LLVM 23.1.6
/// `llvm-mc -show-encoding`. A zero extension always produces a zero-extended
/// value, so the 32-bit form clears the upper half of rax while the 64-bit
/// form produces the same 64-bit value.
#[test]
fn test_movzx_register_source_zero_extends() {
    // mov rbx, 0xffffffffffffffff (48 BB <imm64>)
    let mut code = vec![0x48, 0xBB];
    code.extend_from_slice(&u64::MAX.to_le_bytes());
    code.extend_from_slice(&[0x0F, 0xB6, 0xC3]); // movzbl eax, bl
    code.extend_from_slice(&[0x48, 0x0F, 0xB6, 0xD3]); // movzbq rdx, bl
    code.push(0xF4); // hlt

    let (state, reason) = run_interpreter(&code);
    assert_eq!(reason, ExitReason::Halt);
    // The 32-bit destination holds a zero-extended byte: 0x00000000000000ff.
    assert_eq!(state.get(Register::Rax), 0x0000_0000_0000_00ff);
    // The 64-bit destination holds the same zero-extended value.
    assert_eq!(state.get(Register::Rdx), 0x0000_0000_0000_00ff);
}

/// MOVSX with a register source, for both destination widths.
///
/// The encodings are `movsbl eax, bl` (0F BE C3) and
/// `movsbq rdx, bl` (48 0F BE D3), as produced by LLVM 23.1.6
/// `llvm-mc -show-encoding`. A sign extension reaches the destination width,
/// so the 32-bit form keeps the sign in bits 31:0 and clears the upper half,
/// while the 64-bit form sign-extends across all 64 bits.
#[test]
fn test_movsx_register_source_sign_extends_to_the_destination_width() {
    // mov rbx, 0xffffffffffffffff (48 BB <imm64>)
    let mut code = vec![0x48, 0xBB];
    code.extend_from_slice(&u64::MAX.to_le_bytes());
    code.extend_from_slice(&[0x0F, 0xBE, 0xC3]); // movsbl eax, bl
    code.extend_from_slice(&[0x48, 0x0F, 0xBE, 0xD3]); // movsbq rdx, bl
    code.push(0xF4); // hlt

    let (state, reason) = run_interpreter(&code);
    assert_eq!(reason, ExitReason::Halt);
    // The 32-bit destination holds 0xffffffff and its upper half is cleared.
    assert_eq!(state.get(Register::Rax), 0x0000_0000_ffff_ffff);
    // The 64-bit destination keeps the sign extension across all 64 bits.
    assert_eq!(state.get(Register::Rdx), 0xffff_ffff_ffff_ffff);
}

/// MOVSX from a 16-bit source sign-extends to the destination width.
///
/// The encodings are `movswl eax, bx` (0F BF C3) and
/// `movswq rdx, bx` (48 0F BF D3), as produced by LLVM 23.1.6
/// `llvm-mc -show-encoding`.
#[test]
fn test_movsx_word_register_source_sign_extends() {
    // mov rbx, 0xffffffffffffffff (48 BB <imm64>)
    let mut code = vec![0x48, 0xBB];
    code.extend_from_slice(&u64::MAX.to_le_bytes());
    code.extend_from_slice(&[0x0F, 0xBF, 0xC3]); // movswl eax, bx
    code.extend_from_slice(&[0x48, 0x0F, 0xBF, 0xD3]); // movswq rdx, bx
    code.push(0xF4); // hlt

    let (state, reason) = run_interpreter(&code);
    assert_eq!(reason, ExitReason::Halt);
    assert_eq!(state.get(Register::Rax), 0x0000_0000_ffff_ffff);
    assert_eq!(state.get(Register::Rdx), 0xffff_ffff_ffff_ffff);
}

/// MOVZX with a memory source reads only the low byte of the operand.
///
/// The encodings are `movzbl eax, byte [rdi]` (0F B6 07) and
/// `movzbq rdx, byte [rdi]` (48 0F B6 17), as produced by LLVM 23.1.6
/// `llvm-mc -show-encoding`.
#[test]
fn test_movzx_memory_source_reads_one_byte() {
    // mov rdi, DATA_BASE (48 BF <imm64>)
    // movzbl eax, byte [rdi] (0F B6 07)
    // movzbq rdx, byte [rdi] (48 0F B6 17)
    let code = program_with_rdi(DATA_BASE, &[0x0F, 0xB6, 0x07, 0x48, 0x0F, 0xB6, 0x17]);

    let pe_bytes = build_synthetic_pe32_plus(&code);
    let parsed = ParsedPe::parse(&pe_bytes).expect("parse");
    let mut loaded = parsed.load_into_memory(&pe_bytes).expect("load");
    loaded
        .memory
        .map_region(STACK_BASE, 0x10000, MemoryPermissions::READ_WRITE)
        .expect("stack mapping should succeed");
    loaded
        .memory
        .map_region(DATA_BASE, 0x10000, MemoryPermissions::READ_WRITE)
        .expect("data mapping should succeed");
    // The byte at the source address has the high bit set, so a sign
    // extension would produce a negative value and a zero extension must not.
    loaded
        .memory
        .write_u8(DATA_BASE, 0xFF)
        .expect("source byte");

    let cpu = CpuState::new(loaded.entry_point, STACK_BASE + 0x8000);
    let mut interp = Interpreter::new(cpu, &mut loaded.memory, 100);
    let result = interp.run();

    assert_eq!(result.exit_reason, ExitReason::Halt);
    assert_eq!(result.final_state.get(Register::Rax), 0x0000_0000_0000_00ff);
    assert_eq!(result.final_state.get(Register::Rdx), 0x0000_0000_0000_00ff);
}

/// MOVSX with a memory source sign-extends the byte that was read.
///
/// The encodings are `movsbl eax, byte [rdi]` (0F BE 07) and
/// `movsbq rdx, byte [rdi]` (48 0F BE 17), as produced by LLVM 23.1.6
/// `llvm-mc -show-encoding`.
#[test]
fn test_movsx_memory_source_sign_extends() {
    // mov rdi, DATA_BASE (48 BF <imm64>)
    // movsbl eax, byte [rdi] (0F BE 07)
    // movsbq rdx, byte [rdi] (48 0F BE 17)
    let code = program_with_rdi(DATA_BASE, &[0x0F, 0xBE, 0x07, 0x48, 0x0F, 0xBE, 0x17]);

    let pe_bytes = build_synthetic_pe32_plus(&code);
    let parsed = ParsedPe::parse(&pe_bytes).expect("parse");
    let mut loaded = parsed.load_into_memory(&pe_bytes).expect("load");
    loaded
        .memory
        .map_region(STACK_BASE, 0x10000, MemoryPermissions::READ_WRITE)
        .expect("stack mapping should succeed");
    loaded
        .memory
        .map_region(DATA_BASE, 0x10000, MemoryPermissions::READ_WRITE)
        .expect("data mapping should succeed");
    loaded
        .memory
        .write_u8(DATA_BASE, 0xFF)
        .expect("source byte");

    let cpu = CpuState::new(loaded.entry_point, STACK_BASE + 0x8000);
    let mut interp = Interpreter::new(cpu, &mut loaded.memory, 100);
    let result = interp.run();

    assert_eq!(result.exit_reason, ExitReason::Halt);
    // The 32-bit destination holds 0xffffffff with the upper half cleared.
    assert_eq!(result.final_state.get(Register::Rax), 0x0000_0000_ffff_ffff);
    // The 64-bit destination keeps the sign extension across all 64 bits.
    assert_eq!(result.final_state.get(Register::Rdx), 0xffff_ffff_ffff_ffff);
}

/// The interpreter must implement the same extension opcodes as the native
/// DBT translator.
///
/// The program exercises all four opcodes with both a register and a memory
/// source, and both destination widths:
///
/// - `movzbl eax, bl` (0F B6 C3), `movzbq rdx, bl` (48 0F B6 D3)
/// - `movswl eax, bx` (0F BF C3), `movswq rdx, bx` (48 0F BF D3)
/// - `movzbl eax, byte [rdi]` (0F B6 07), `movzbq rdx, byte [rdi]`
///   (48 0F B6 17)
/// - `movsbl eax, byte [rdi]` (0F BE 07), `movsbq rdx, byte [rdi]`
///   (48 0F BE 17)
///
/// All encodings were produced by LLVM 23.1.6 `llvm-mc -show-encoding`.
///
/// The expected values follow the Intel SDM rules for MOVZX and MOVSX: MOVZX
/// always zero-extends, and MOVSX sign-extends to the destination width, so a
/// 32-bit destination keeps the sign in bits 31:0 and clears the upper half
/// while a 64-bit destination keeps the sign extension across all 64 bits.
#[test]
fn test_interpreter_implements_the_extension_opcodes() {
    let code = program_with_rdi(
        DATA_BASE,
        &[
            0x0F, 0xB6, 0xC3, // movzbl eax, bl
            0x48, 0x0F, 0xB6, 0xD3, // movzbq rdx, bl
            0x0F, 0xBF, 0xC3, // movswl eax, bx
            0x48, 0x0F, 0xBF, 0xD3, // movswq rdx, bx
            0x0F, 0xB6, 0x07, // movzbl eax, byte [rdi]
            0x48, 0x0F, 0xB6, 0x17, // movzbq rdx, byte [rdi]
            0x0F, 0xBE, 0x07, // movsbl eax, byte [rdi]
            0x48, 0x0F, 0xBE, 0x17, // movsbq rdx, byte [rdi]
        ],
    );

    let pe_bytes = build_synthetic_pe32_plus(&code);
    let parsed = ParsedPe::parse(&pe_bytes).expect("parse");
    let mut loaded = parsed.load_into_memory(&pe_bytes).expect("load");
    loaded
        .memory
        .map_region(STACK_BASE, 0x10000, MemoryPermissions::READ_WRITE)
        .expect("stack mapping should succeed");
    loaded
        .memory
        .map_region(DATA_BASE, 0x10000, MemoryPermissions::READ_WRITE)
        .expect("data mapping should succeed");
    // The bytes at the source address have the high bit set and clear, so a
    // sign extension and a zero extension give different results.
    loaded
        .memory
        .write_u8(DATA_BASE, 0xFF)
        .expect("source byte");
    loaded
        .memory
        .write_u8(DATA_BASE + 1, 0x7F)
        .expect("source byte");
    let mut cpu = CpuState::new(loaded.entry_point, STACK_BASE + 0x8000);
    // rbx and rsi hold the register-source values.
    cpu.set(Register::Rbx, u64::MAX);
    cpu.set(Register::Rsi, 0x0000_0000_0000_7FFF);
    let mut interp = Interpreter::new(cpu, &mut loaded.memory, 100);
    let result = interp.run();

    assert_eq!(result.exit_reason, ExitReason::Halt);
    // The last write to rax is the 32-bit sign extension of the byte 0xFF.
    assert_eq!(result.final_state.get(Register::Rax), 0x0000_0000_ffff_ffff);
    // The last write to rdx is the 64-bit sign extension of the same byte.
    assert_eq!(result.final_state.get(Register::Rdx), 0xffff_ffff_ffff_ffff);
}
