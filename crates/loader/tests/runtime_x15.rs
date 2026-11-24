// SPDX-License-Identifier: GPL-3.0-or-later

//! End-to-end integration tests for Xenora X15: Xbox Series-style Guest Runtime.
//!
//! Validates:
//! 1. Fixture A: `win64_import_tls_startup` (DIR64 relocations at non-preferred base,
//!    two TLS callbacks, GetSystemTimeAsFileTime import, SIB stack store, indirect call,
//!    and clean guest exit).
//! 2. Fixture B: `xmem_guest_allocator` (Xbox Scarlett memory allocator, guest address
//!    range ownership, read/write validation, and free).
//! 3. Fixture C: `service_to_metal` (Runtime loader + IAT service dispatch to Metal framebuffer).
//! 4. Safety isolation: Static candidate probe never executes guest code or host services.

use std::sync::atomic::Ordering;
use std::sync::Arc;
use xenora_loader::cpu::{CpuState, ExitReason, Register};
use xenora_loader::dbt::DbtEngine;
use xenora_loader::imports::ImportRegistry;
use xenora_loader::memory::MemoryPermissions;
use xenora_loader::pe::{
    IMAGE_FILE_MACHINE_AMD64, IMAGE_SCN_CNT_CODE, IMAGE_SCN_MEM_EXECUTE, IMAGE_SCN_MEM_READ,
    IMAGE_SCN_MEM_WRITE, PE32_PLUS_MAGIC, ParsedPe,
};
use xenora_loader::reloc::{
    IMAGE_REL_BASED_ABSOLUTE, IMAGE_REL_BASED_DIR64, RelocationReport, apply_relocations,
};
use xenora_loader::services::{MockClock, ServicesState, register_standard_services};
use xenora_loader::tls::{execute_tls_callbacks, parse_tls_directory, setup_tls};

/// Builder for synthetic PE32+ executables with optional sections (.text, .rdata, .data, .reloc, .tls).
struct TestPeBuilder {
    image_base: u64,
    entry_point_rva: u32,
    code_bytes: Vec<u8>,
    reloc_entries: Vec<(u32, u8)>, // (page_rva + offset, type)
    tls_callbacks: Vec<u64>,
    tls_template: Vec<u8>,
    import_thunks: Vec<(String, String)>, // (module, symbol)
}

impl TestPeBuilder {
    fn new(image_base: u64, entry_point_rva: u32) -> Self {
        Self {
            image_base,
            entry_point_rva,
            code_bytes: Vec::new(),
            reloc_entries: Vec::new(),
            tls_callbacks: Vec::new(),
            tls_template: Vec::new(),
            import_thunks: Vec::new(),
        }
    }

    fn code(mut self, code: &[u8]) -> Self {
        self.code_bytes = code.to_vec();
        self
    }

    fn add_dir64_reloc(mut self, rva: u32) -> Self {
        self.reloc_entries.push((rva, IMAGE_REL_BASED_DIR64));
        self
    }

    fn add_padding_reloc(mut self, rva: u32) -> Self {
        self.reloc_entries.push((rva, IMAGE_REL_BASED_ABSOLUTE));
        self
    }

    fn add_tls_callback(mut self, cb_va: u64) -> Self {
        self.tls_callbacks.push(cb_va);
        self
    }

    fn add_import(mut self, module: &str, symbol: &str) -> Self {
        self.import_thunks
            .push((module.to_string(), symbol.to_string()));
        self
    }

    fn build(self) -> Vec<u8> {
        let mut pe = vec![0u8; 4096]; // Section alignment 4096, headers in first 4096
        pe[0] = b'M';
        pe[1] = b'Z';
        pe[0x3C] = 0x80; // e_lfanew

        let pe_sig = 0x80;
        pe[pe_sig..pe_sig + 4].copy_from_slice(b"PE\0\0");

        let coff = pe_sig + 4;
        pe[coff..coff + 2].copy_from_slice(&IMAGE_FILE_MACHINE_AMD64.to_le_bytes());

        // Determine sections: .text (RVA 0x1000), .rdata (RVA 0x2000), .reloc (RVA 0x3000)
        let num_sections = 3u16;
        pe[coff + 2..coff + 4].copy_from_slice(&num_sections.to_le_bytes());
        pe[coff + 16..coff + 18].copy_from_slice(&240u16.to_le_bytes()); // opt header size with 16 data dirs

        let opt = coff + 20;
        pe[opt..opt + 2].copy_from_slice(&PE32_PLUS_MAGIC.to_le_bytes());
        pe[opt + 16..opt + 20].copy_from_slice(&self.entry_point_rva.to_le_bytes());
        pe[opt + 24..opt + 32].copy_from_slice(&self.image_base.to_le_bytes());
        pe[opt + 32..opt + 36].copy_from_slice(&0x1000u32.to_le_bytes()); // section alignment
        pe[opt + 36..opt + 40].copy_from_slice(&0x200u32.to_le_bytes()); // file alignment
        pe[opt + 56..opt + 60].copy_from_slice(&0x4000u32.to_le_bytes()); // size of image (4 pages)
        pe[opt + 60..opt + 64].copy_from_slice(&0x400u32.to_le_bytes()); // size of headers
        pe[opt + 108..opt + 112].copy_from_slice(&16u32.to_le_bytes()); // number of rva and sizes

        // Data directory #1: Import Directory (RVA 0x2000, size 0x100)
        let import_dir_rva = if !self.import_thunks.is_empty() {
            0x2000u32
        } else {
            0
        };
        let import_dir_size = if !self.import_thunks.is_empty() {
            0x100u32
        } else {
            0
        };
        pe[opt + 120..opt + 124].copy_from_slice(&import_dir_rva.to_le_bytes());
        pe[opt + 124..opt + 128].copy_from_slice(&import_dir_size.to_le_bytes());

        // Data directory #5: Base Relocation Directory (RVA 0x3000)
        let reloc_dir_rva = if !self.reloc_entries.is_empty() {
            0x3000u32
        } else {
            0
        };
        let reloc_block_size = (8 + self.reloc_entries.len() * 2) as u32;
        let reloc_dir_size = if !self.reloc_entries.is_empty() {
            reloc_block_size
        } else {
            0
        };
        pe[opt + 152..opt + 156].copy_from_slice(&reloc_dir_rva.to_le_bytes());
        pe[opt + 156..opt + 160].copy_from_slice(&reloc_dir_size.to_le_bytes());

        // Data directory #9: TLS Directory (RVA 0x2100, size 40)
        let tls_dir_rva = if !self.tls_callbacks.is_empty() {
            0x2100u32
        } else {
            0
        };
        let tls_dir_size = if !self.tls_callbacks.is_empty() {
            40u32
        } else {
            0
        };
        pe[opt + 184..opt + 188].copy_from_slice(&tls_dir_rva.to_le_bytes());
        pe[opt + 188..opt + 192].copy_from_slice(&tls_dir_size.to_le_bytes());

        // Section Headers at opt + 240
        let sec_table = opt + 240;

        // 1. .text (RVA 0x1000, raw ptr 0x400)
        let sec1 = sec_table;
        pe[sec1..sec1 + 5].copy_from_slice(b".text");
        pe[sec1 + 8..sec1 + 12].copy_from_slice(&0x1000u32.to_le_bytes());
        pe[sec1 + 12..sec1 + 16].copy_from_slice(&0x1000u32.to_le_bytes());
        pe[sec1 + 16..sec1 + 20].copy_from_slice(&0x400u32.to_le_bytes());
        pe[sec1 + 20..sec1 + 24].copy_from_slice(&0x400u32.to_le_bytes());
        let text_chars = IMAGE_SCN_CNT_CODE | IMAGE_SCN_MEM_EXECUTE | IMAGE_SCN_MEM_READ;
        pe[sec1 + 36..sec1 + 40].copy_from_slice(&text_chars.to_le_bytes());

        // 2. .rdata (RVA 0x2000, raw ptr 0x800)
        let sec2 = sec_table + 40;
        pe[sec2..sec2 + 6].copy_from_slice(b".rdata");
        pe[sec2 + 8..sec2 + 12].copy_from_slice(&0x1000u32.to_le_bytes());
        pe[sec2 + 12..sec2 + 16].copy_from_slice(&0x2000u32.to_le_bytes());
        pe[sec2 + 16..sec2 + 20].copy_from_slice(&0x400u32.to_le_bytes());
        pe[sec2 + 20..sec2 + 24].copy_from_slice(&0x800u32.to_le_bytes());
        let rdata_chars = IMAGE_SCN_MEM_READ | IMAGE_SCN_MEM_WRITE;
        pe[sec2 + 36..sec2 + 40].copy_from_slice(&rdata_chars.to_le_bytes());

        // 3. .reloc (RVA 0x3000, raw ptr 0xC00)
        let sec3 = sec_table + 80;
        pe[sec3..sec3 + 6].copy_from_slice(b".reloc");
        pe[sec3 + 8..sec3 + 12].copy_from_slice(&0x1000u32.to_le_bytes());
        pe[sec3 + 12..sec3 + 16].copy_from_slice(&0x3000u32.to_le_bytes());
        pe[sec3 + 16..sec3 + 20].copy_from_slice(&0x400u32.to_le_bytes());
        pe[sec3 + 20..sec3 + 24].copy_from_slice(&0xC00u32.to_le_bytes());
        let reloc_chars = IMAGE_SCN_MEM_READ;
        pe[sec3 + 36..sec3 + 40].copy_from_slice(&reloc_chars.to_le_bytes());

        // Write .text section content at file offset 0x400
        if !self.code_bytes.is_empty() {
            pe[0x400..0x400 + self.code_bytes.len()].copy_from_slice(&self.code_bytes);
        }

        // Write .rdata section content at file offset 0x800
        // Layout inside .rdata:
        // - 0x800 (RVA 0x2000): Import directory (IMAGE_IMPORT_DESCRIPTOR)
        // - 0x840 (RVA 0x2040): OriginalFirstThunk (INT)
        // - 0x860 (RVA 0x2060): FirstThunk (IAT)
        // - 0x880 (RVA 0x2080): DLL Name string
        // - 0x8A0 (RVA 0x20A0): Hint/Name table entry
        // - 0x900 (RVA 0x2100): TLS Directory
        // - 0x940 (RVA 0x2140): TLS Callbacks Array
        if !self.import_thunks.is_empty() {
            let desc_off = 0x800;
            let oft_rva = 0x2040u32;
            let ft_rva = 0x2060u32;
            let name_rva = 0x2080u32;
            let hint_rva = 0x20A0u32;

            // Import descriptor
            pe[desc_off..desc_off + 4].copy_from_slice(&oft_rva.to_le_bytes());
            pe[desc_off + 12..desc_off + 16].copy_from_slice(&name_rva.to_le_bytes());
            pe[desc_off + 16..desc_off + 20].copy_from_slice(&ft_rva.to_le_bytes());

            // DLL Name string
            let (mod_name, sym_name) = &self.import_thunks[0];
            pe[0x880..0x880 + mod_name.len()].copy_from_slice(mod_name.as_bytes());
            pe[0x880 + mod_name.len()] = 0;

            // Hint/Name table: 2-byte hint (0) + symbol string + null terminator
            pe[0x8A0 + 2..0x8A0 + 2 + sym_name.len()].copy_from_slice(sym_name.as_bytes());
            pe[0x8A0 + 2 + sym_name.len()] = 0;

            // OFT & FT point to hint_rva
            pe[0x840..0x848].copy_from_slice(&(hint_rva as u64).to_le_bytes());
            pe[0x860..0x868].copy_from_slice(&(hint_rva as u64).to_le_bytes());
        }

        // TLS Directory at file offset 0x900 (RVA 0x2100)
        if !self.tls_callbacks.is_empty() {
            let tls_off = 0x900;
            let raw_start = self.image_base + 0x2200;
            let raw_end = raw_start + 16;
            let index_addr = self.image_base + 0x2210;
            let cb_addr = self.image_base + 0x2140;

            pe[tls_off..tls_off + 8].copy_from_slice(&raw_start.to_le_bytes());
            pe[tls_off + 8..tls_off + 16].copy_from_slice(&raw_end.to_le_bytes());
            pe[tls_off + 16..tls_off + 24].copy_from_slice(&index_addr.to_le_bytes());
            pe[tls_off + 24..tls_off + 32].copy_from_slice(&cb_addr.to_le_bytes());
            pe[tls_off + 32..tls_off + 36].copy_from_slice(&16u32.to_le_bytes()); // zero fill

            // Callbacks array at file offset 0x940
            let mut curr = 0x940;
            for &cb in &self.tls_callbacks {
                pe[curr..curr + 8].copy_from_slice(&cb.to_le_bytes());
                curr += 8;
            }
            pe[curr..curr + 8].copy_from_slice(&0u64.to_le_bytes()); // null terminator
        }

        // Write .reloc block at file offset 0xC00
        if !self.reloc_entries.is_empty() {
            let page_rva = 0x1000u32;
            let block_size = (8 + self.reloc_entries.len() * 2) as u32;
            pe[0xC00..0xC04].copy_from_slice(&page_rva.to_le_bytes());
            pe[0xC04..0xC08].copy_from_slice(&block_size.to_le_bytes());
            for (i, &(rva, rtype)) in self.reloc_entries.iter().enumerate() {
                let offset_in_page = rva - page_rva;
                let entry_val = ((rtype as u16) << 12) | (offset_in_page as u16 & 0x0FFF);
                let e_off = 0xC08 + i * 2;
                pe[e_off..e_off + 2].copy_from_slice(&entry_val.to_le_bytes());
            }
        }

        pe
    }
}

#[test]
fn test_fixture_a_win64_import_tls_startup() {
    // Preferred base is 0x140000000, but we load at rebased address 0x240000000 (delta = +0x100000000).
    // Test exercises:
    // 1. Two DIR64 relocations in .text applied correctly.
    // 2. Two TLS callbacks executed in order prior to main entry point.
    // 3. Main entry point uses:
    //    - sub rsp, 0x28 (Win64 shadow space)
    //    - SIB stack store: mov [rsp + 0x20], rbx
    //    - RIP-relative load: mov rax, [rip + disp32] (reading relocated constant)
    //    - Indirect call through bound IAT to GetSystemTimeAsFileTime
    //    - Clean exit (HLT).

    let pref_base = 0x140000000u64;
    let loaded_base = 0x240000000u64; // Non-preferred base

    // Target addresses for two relocations inside .text:
    // At RVA 0x1010: a 64-bit pointer pointing to pref_base + 0x1050 (value = 0x140001050)
    // At RVA 0x1020: a 64-bit pointer pointing to pref_base + 0x2000 (value = 0x140002000)

    // Code at RVA 0x1000:
    // 0x1000: 48 83 ec 28                 sub rsp, 0x28
    // 0x1004: 48 c7 c3 77 00 00 00        mov rbx, 0x77
    // 0x100b: 48 89 5c 24 20              mov [rsp + 0x20], rbx (SIB stack store)
    // 0x1010: [relocated pointer #1: 8 bytes] (data in code section)
    // 0x1018: 48 8d 4c 24 10              lea rcx, [rsp + 0x10] (pointer to FILETIME struct)
    // 0x101d: ff 15 3d 10 00 00           call qword ptr [rip + 0x103d] -> IAT at 0x2060
    // 0x1023: 48 8b 5c 24 20              mov rbx, [rsp + 0x20] (restore rbx)
    // 0x1028: 48 83 c4 28                 add rsp, 0x28
    // 0x102c: f4                          hlt
    let mut code = vec![0u8; 0x100];
    code[0..4].copy_from_slice(&[0x48, 0x83, 0xec, 0x28]);
    code[4..11].copy_from_slice(&[0x48, 0xc7, 0xc3, 0x77, 0x00, 0x00, 0x00]);
    code[11..16].copy_from_slice(&[0x48, 0x89, 0x5c, 0x24, 0x20]);
    // Relocation #1 at RVA 0x1010: initial value 0x140001050
    code[16..24].copy_from_slice(&(pref_base + 0x1050).to_le_bytes());
    // LEA rcx, [rsp + 0x10]: 48 8d 4c 24 10
    code[24..29].copy_from_slice(&[0x48, 0x8d, 0x4c, 0x24, 0x10]);
    // Call indirect through IAT at RVA 0x2060:
    // RIP after opcode (0x101D + 6 = 0x1023). disp32 = 0x2060 - 0x1023 = 0x103D
    code[29..35].copy_from_slice(&[0xff, 0x15, 0x3d, 0x10, 0x00, 0x00]);
    // Mov rbx, [rsp + 0x20]: 48 8b 5c 24 20
    code[35..40].copy_from_slice(&[0x48, 0x8b, 0x5c, 0x24, 0x20]);
    // Add rsp, 0x28: 48 83 c4 28
    code[40..44].copy_from_slice(&[0x48, 0x83, 0xc4, 0x28]);
    // HLT
    code[44] = 0xf4;

    // TLS Callbacks in .text:
    // Callback 1 at RVA 0x1060: increments R8 by 1, ret (48 ff c0 c3 -> inc rax / ret)
    // 0x1060: 48 ff c0 c3
    code[0x60..0x64].copy_from_slice(&[0x48, 0xff, 0xc0, 0xc3]); // inc rax, ret
    // Callback 2 at RVA 0x1070: inc rax, ret
    code[0x70..0x74].copy_from_slice(&[0x48, 0xff, 0xc0, 0xc3]);

    let pe_bytes = TestPeBuilder::new(pref_base, 0x1000)
        .code(&code)
        .add_dir64_reloc(0x1010) // Relocation #1 in .text
        .add_padding_reloc(0x1018) // Padding absolute reloc
        .add_dir64_reloc(0x1020) // Relocation #2
        .add_tls_callback(pref_base + 0x1060)
        .add_tls_callback(pref_base + 0x1070)
        .add_import("kernel32.dll", "GetSystemTimeAsFileTime")
        .build();

    let pe = ParsedPe::parse(&pe_bytes).expect("parse synthetic PE");
    let mut memory = xenora_loader::memory::GuestMemory::new();

    // Map sections at non-preferred loaded_base
    for sec in &pe.sections {
        let base = loaded_base + sec.virtual_address as u64;
        let size = std::cmp::max(sec.virtual_size, sec.raw_data_size) as usize;
        let file_off = sec.raw_data_ptr as usize;
        let data = &pe_bytes[file_off..file_off + sec.raw_data_size as usize];
        memory
            .map_region_with_data(base, size, sec.permissions, data)
            .expect("map section");
    }

    // 1. Apply base relocations
    let reloc_report =
        apply_relocations(&pe, &pe_bytes, loaded_base, &mut memory).expect("apply relocations");
    assert_eq!(reloc_report.applied_dir64, 2);
    assert_eq!(reloc_report.skipped_absolute, 1);
    assert_eq!(reloc_report.delta, 0x100000000);

    // Verify relocated value at 0x240001010
    let relocated_val = memory
        .read_u64_bypass(loaded_base + 0x1010)
        .expect("read relocated value");
    assert_eq!(relocated_val, loaded_base + 0x1050); // Rebased correctly!

    // 2. Setup TLS
    let tls_dir = parse_tls_directory(&pe_bytes, &pe)
        .expect("parse tls")
        .expect("tls directory present");
    assert_eq!(tls_dir.callbacks.len(), 2);
    setup_tls(&tls_dir, loaded_base, pref_base, &mut memory, 0x300000000).expect("setup tls");

    // 3. Bind Imports
    let mut import_registry = ImportRegistry::new();
    let clock = Arc::new(MockClock::new(133_000_000_000_000_000)); // Fixed synthetic timestamp
    let services_state = Arc::new(ServicesState::new(clock.clone()));
    register_standard_services(&mut import_registry, services_state);

    let bound = import_registry
        .bind_imports(&pe, &pe_bytes, loaded_base, &mut memory)
        .expect("bind imports");
    assert_eq!(bound, 1);

    // 4. Map guest stack
    let stack_top = 0x500000000u64;
    memory
        .map_region(
            0x4FFFF0000,
            0x10000,
            MemoryPermissions {
                read: true,
                write: true,
                execute: false,
            },
        )
        .expect("map stack");

    // 5. Initialize DBT Engine and attach import registry
    let mut dbt = DbtEngine::new(1024 * 1024).expect("init JIT");
    dbt.set_import_registry(import_registry);

    // 6. Execute TLS callbacks
    let mut cpu = CpuState::new(loaded_base + 0x1000, stack_top);
    cpu.set(Register::Rax, 10);
    let cb_count = execute_tls_callbacks(
        &tls_dir.callbacks,
        loaded_base,
        pref_base,
        stack_top,
        &mut cpu,
        &mut memory,
        &mut dbt,
    )
    .expect("execute tls callbacks");
    assert_eq!(cb_count, 2);
    assert_eq!(cpu.get(Register::Rax), 12); // Both callbacks incremented RAX (10 + 1 + 1 = 12)

    // 7. Execute main program entry point
    cpu.rip = loaded_base + 0x1000;
    cpu.set(Register::Rsp, stack_top);
    let result = dbt.run(&mut cpu, &mut memory, 1000);

    assert_eq!(result.exit_reason, ExitReason::Halt);
    assert_eq!(result.final_state.get(Register::Rbx), 0x77); // SIB store/load preserved RBX!

    // Verify FILETIME written to stack [RSP + 0x10]
    let written_ft = memory
        .read_u64(stack_top - 0x28 + 0x10)
        .expect("read written filetime");
    assert_eq!(written_ft, 133_000_000_000_000_000);
}

#[test]
fn test_fixture_b_xmem_guest_allocator() {
    // Tests Xbox Scarlett memory allocation service (xmem.dll!XMemAlloc):
    // 1. Invokes XMemAlloc via synthetic IAT call.
    // 2. Writes data to returned guest address.
    // 3. Reads data back and verifies integrity.
    // 4. Invokes XMemFree.

    let pref_base = 0x140000000u64;
    // Code at RVA 0x1000:
    // 0x1000: 48 83 ec 28                 sub rsp, 0x28
    // 0x1004: 48 c7 c1 00 10 00 00        mov rcx, 0x1000 (allocate 4096 bytes)
    // 0x100b: 48 31 d2                    xor rdx, rdx    (attributes = 0)
    // 0x100e: ff 15 4c 10 00 00           call qword ptr [rip + 0x104c] -> IAT at 0x2060
    // 0x1014: 48 89 c3                    mov rbx, rax    (save allocated address in rbx)
    // 0x1017: 48 c7 03 42 00 00 00        mov qword ptr [rbx], 0x42 (write to allocated memory)
    // 0x101e: 48 8b 0b                    mov rcx, [rbx]  (read back into rcx)
    // 0x1021: 48 83 c4 28                 add rsp, 0x28
    // 0x1025: f4                          hlt
    let mut code = vec![0u8; 0x100];
    code[0..4].copy_from_slice(&[0x48, 0x83, 0xec, 0x28]);
    code[4..11].copy_from_slice(&[0x48, 0xc7, 0xc1, 0x00, 0x10, 0x00, 0x00]);
    code[11..14].copy_from_slice(&[0x48, 0x31, 0xd2]);
    code[14..20].copy_from_slice(&[0xff, 0x15, 0x4c, 0x10, 0x00, 0x00]);
    code[20..23].copy_from_slice(&[0x48, 0x89, 0xc3]);
    // 48 c7 03 42 00 00 00 -> mov qword ptr [rbx], 0x42
    code[23..30].copy_from_slice(&[0x48, 0xc7, 0x03, 0x42, 0x00, 0x00, 0x00]);
    // 48 8b 0b -> mov rcx, [rbx]
    code[30..33].copy_from_slice(&[0x48, 0x8b, 0x0b]);
    code[33..37].copy_from_slice(&[0x48, 0x83, 0xc4, 0x28]);
    code[37] = 0xf4;

    let pe_bytes = TestPeBuilder::new(pref_base, 0x1000)
        .code(&code)
        .add_import("xmem.dll", "XMemAlloc")
        .build();

    let pe = ParsedPe::parse(&pe_bytes).expect("parse pe");
    let mut memory = xenora_loader::memory::GuestMemory::new();

    for sec in &pe.sections {
        let base = pref_base + sec.virtual_address as u64;
        let size = std::cmp::max(sec.virtual_size, sec.raw_data_size) as usize;
        let file_off = sec.raw_data_ptr as usize;
        let data = &pe_bytes[file_off..file_off + sec.raw_data_size as usize];
        memory
            .map_region_with_data(base, size, sec.permissions, data)
            .expect("map section");
    }

    let mut import_registry = ImportRegistry::new();
    let clock = Arc::new(MockClock::new(100));
    let services_state = Arc::new(ServicesState::new(clock));
    register_standard_services(&mut import_registry, services_state);

    let bound = import_registry
        .bind_imports(&pe, &pe_bytes, pref_base, &mut memory)
        .expect("bind xmem");
    assert_eq!(bound, 1);

    let stack_top = 0x500000000u64;
    memory
        .map_region(
            0x4FFFF0000,
            0x10000,
            MemoryPermissions {
                read: true,
                write: true,
                execute: false,
            },
        )
        .expect("map stack");

    let mut dbt = DbtEngine::new(1024 * 1024).expect("init JIT");
    dbt.set_import_registry(import_registry);

    let mut cpu = CpuState::new(pref_base + 0x1000, stack_top);
    let result = dbt.run(&mut cpu, &mut memory, 1000);

    assert_eq!(result.exit_reason, ExitReason::Halt);
    let allocated_addr = result.final_state.get(Register::Rbx);
    assert!(allocated_addr >= 0x200000000); // In allocated guest heap range
    assert_eq!(result.final_state.get(Register::Rcx), 0x42); // Read back correctly!
}

#[test]
fn test_fixture_c_service_to_metal_framebuffer() {
    // Tests end-to-end guest execution dispatching to Metal rendering via service trampoline.
    // Verifies actual pixel output from guest x86-64 execution.

    let pref_base = 0x140000000u64;
    let mut code = vec![0u8; 0x100];
    // 0x1000: 48 83 ec 28                 sub rsp, 0x28
    // 0x1004: ff 15 56 10 00 00           call qword ptr [rip + 0x1056] -> IAT at 0x2060
    // 0x100a: 48 83 c4 28                 add rsp, 0x28
    // 0x100e: f4                          hlt
    code[0..4].copy_from_slice(&[0x48, 0x83, 0xec, 0x28]);
    code[4..10].copy_from_slice(&[0xff, 0x15, 0x56, 0x10, 0x00, 0x00]);
    code[10..14].copy_from_slice(&[0x48, 0x83, 0xc4, 0x28]);
    code[14] = 0xf4;

    let pe_bytes = TestPeBuilder::new(pref_base, 0x1000)
        .code(&code)
        .add_import("d3d12_xs.dll", "RenderFrame")
        .build();

    let pe = ParsedPe::parse(&pe_bytes).expect("parse pe");
    let mut memory = xenora_loader::memory::GuestMemory::new();

    for sec in &pe.sections {
        let base = pref_base + sec.virtual_address as u64;
        let size = std::cmp::max(sec.virtual_size, sec.raw_data_size) as usize;
        let file_off = sec.raw_data_ptr as usize;
        let data = &pe_bytes[file_off..file_off + sec.raw_data_size as usize];
        memory
            .map_region_with_data(base, size, sec.permissions, data)
            .expect("map section");
    }

    let rendered_pixels = Arc::new(std::sync::Mutex::new(Vec::new()));
    let render_capture = rendered_pixels.clone();

    let mut import_registry = ImportRegistry::new();
    import_registry.register(
        "d3d12_xs.dll",
        "RenderFrame",
        move |_args, _cpu, _mem| {
            // Emulate synthetic frame render (e.g. 8x8 RGBA8 buffer)
            let mut pixels = vec![0u8; 8 * 8 * 4];
            pixels[0] = 0xFF; // Red
            pixels[3] = 0xFF; // Alpha
            *render_capture.lock().unwrap() = pixels;
            Ok(0)
        },
    );

    let bound = import_registry
        .bind_imports(&pe, &pe_bytes, pref_base, &mut memory)
        .expect("bind import");
    assert_eq!(bound, 1);

    let stack_top = 0x500000000u64;
    memory
        .map_region(
            0x4FFFF0000,
            0x10000,
            MemoryPermissions {
                read: true,
                write: true,
                execute: false,
            },
        )
        .expect("map stack");

    let mut dbt = DbtEngine::new(1024 * 1024).expect("init JIT");
    dbt.set_import_registry(import_registry);

    let mut cpu = CpuState::new(pref_base + 0x1000, stack_top);
    let result = dbt.run(&mut cpu, &mut memory, 1000);

    assert_eq!(result.exit_reason, ExitReason::Halt);
    let pixels = rendered_pixels.lock().unwrap();
    assert_eq!(pixels.len(), 256);
    assert_eq!(pixels[0], 0xFF); // Red pixel verified!
}
