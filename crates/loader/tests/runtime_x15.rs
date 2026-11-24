// SPDX-License-Identifier: GPL-3.0-or-later

//! End-to-end integration tests for Xenora X15: Xbox Series-style Guest Runtime.
//!
//! Validates:
//! 1. Fixture A: `win64_import_tls_startup` (DIR64 relocations at non-preferred base,
//!    two TLS callbacks, GetSystemTimeAsFileTime import, SIB stack store, indirect call,
//!    and clean guest exit).
//! 2. Fixture B: `xmem_guest_allocator` (Xbox Scarlett memory allocator, guest address
//!    allocation and read/write validation).
//! 3. Fixture C: IAT dispatch to a mock service and, with the GPU gate, native Metal readback.

use std::sync::Arc;
use xenora_loader::cpu::{CpuState, ExitReason, Register};
use xenora_loader::dbt::DbtEngine;
use xenora_loader::imports::ImportRegistry;
use xenora_loader::memory::MemoryPermissions;
use xenora_loader::pe::{
    IMAGE_FILE_MACHINE_AMD64, IMAGE_SCN_CNT_CODE, IMAGE_SCN_MEM_EXECUTE, IMAGE_SCN_MEM_READ,
    IMAGE_SCN_MEM_WRITE, PE32_PLUS_MAGIC, ParsedPe,
};
use xenora_loader::reloc::{IMAGE_REL_BASED_ABSOLUTE, IMAGE_REL_BASED_DIR64};
use xenora_loader::services::{MockClock, ServicesState, register_standard_services};
use xenora_loader::startup::{StartupLayout, run_synthetic_startup};
use xenora_loader::tls::TlsError;

/// Builder for synthetic PE32+ executables with optional sections (.text, .rdata, .data, .reloc, .tls).
struct TestPeBuilder {
    image_base: u64,
    entry_point_rva: u32,
    code_bytes: Vec<u8>,
    reloc_entries: Vec<(u32, u8)>, // (page_rva + offset, type)
    tls_callbacks: Vec<u64>,
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
        assert!(self.import_thunks.len() <= 2);
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
        // - 0x8C0 (RVA 0x20C0): Hint/Name table entry
        // - 0x900 (RVA 0x2100): TLS Directory
        // - 0x940 (RVA 0x2140): TLS Callbacks Array
        if !self.import_thunks.is_empty() {
            let desc_off = 0x800;
            let oft_rva = 0x2040u32;
            let ft_rva = 0x2060u32;
            let name_rva = 0x2080u32;
            let hint_rva = 0x20C0u32;

            // Import descriptor
            pe[desc_off..desc_off + 4].copy_from_slice(&oft_rva.to_le_bytes());
            pe[desc_off + 12..desc_off + 16].copy_from_slice(&name_rva.to_le_bytes());
            pe[desc_off + 16..desc_off + 20].copy_from_slice(&ft_rva.to_le_bytes());

            // DLL Name string
            let (mod_name, sym_name) = &self.import_thunks[0];
            pe[0x880..0x880 + mod_name.len()].copy_from_slice(mod_name.as_bytes());
            pe[0x880 + mod_name.len()] = 0;

            // Hint/Name table: 2-byte hint (0) + symbol string + null terminator
            pe[0x8C0 + 2..0x8C0 + 2 + sym_name.len()].copy_from_slice(sym_name.as_bytes());
            pe[0x8C0 + 2 + sym_name.len()] = 0;

            // OFT & FT point to hint_rva
            pe[0x840..0x848].copy_from_slice(&(hint_rva as u64).to_le_bytes());
            pe[0x860..0x868].copy_from_slice(&(hint_rva as u64).to_le_bytes());
        }

        if let Some((module, symbol)) = self.import_thunks.get(1) {
            let descriptor = 0x814;
            pe[descriptor..descriptor + 4].copy_from_slice(&0x2180u32.to_le_bytes());
            pe[descriptor + 12..descriptor + 16].copy_from_slice(&0x21c0u32.to_le_bytes());
            pe[descriptor + 16..descriptor + 20].copy_from_slice(&0x21a0u32.to_le_bytes());
            pe[0x980..0x988].copy_from_slice(&0x2200u64.to_le_bytes());
            pe[0x9a0..0x9a8].copy_from_slice(&0x2200u64.to_le_bytes());
            pe[0x9c0..0x9c0 + module.len()].copy_from_slice(module.as_bytes());
            pe[0xa02..0xa02 + symbol.len()].copy_from_slice(symbol.as_bytes());
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
    //    - Indirect call through bound IAT to GetSystemTimeAsFileTime
    //    - Clean exit (HLT).

    let pref_base = 0x140000000u64;
    let loaded_base = 0x240000000u64; // Non-preferred base

    // Code layout at RVA 0x1000:
    // 0x1000: 48 83 ec 28                 sub rsp, 0x28
    // 0x1004: 48 c7 c3 77 00 00 00        mov rbx, 0x77
    // 0x100b: 48 89 5c 24 20              mov [rsp + 0x20], rbx (SIB stack store)
    // 0x1010: 48 8d 4c 24 10              lea rcx, [rsp + 0x10] (pointer to FILETIME struct)
    // 0x1015: ff 15 45 10 00 00           call qword ptr [rip + 0x1045] -> IAT at 0x2060 (0x2060 - 0x101B = 0x1045)
    // 0x101b: 48 8b 54 24 20              mov rdx, [rsp + 0x20] (read back saved value)
    // 0x1020: 48 83 c4 28                 add rsp, 0x28
    // 0x1024: f4                          hlt
    // 0x1028: [relocated pointer #1: 8 bytes] (at RVA 0x1028: pref_base + 0x1050)
    // 0x1030: [relocated pointer #2: 8 bytes] (at RVA 0x1030: pref_base + 0x2000)
    let mut code = vec![0u8; 0x100];
    code[0..4].copy_from_slice(&[0x48, 0x83, 0xec, 0x28]);
    code[4..11].copy_from_slice(&[0x48, 0xc7, 0xc3, 0x77, 0x00, 0x00, 0x00]);
    code[11..16].copy_from_slice(&[0x48, 0x89, 0x5c, 0x24, 0x20]);
    // LEA rcx, [rsp + 0x10] (48 8d 4c 24 10)
    code[16..21].copy_from_slice(&[0x48, 0x8d, 0x4c, 0x24, 0x10]);
    // Call indirect through IAT at 0x2060: 0x2060 - (0x1015 + 6 = 0x101B) = 0x1045
    code[21..27].copy_from_slice(&[0xff, 0x15, 0x45, 0x10, 0x00, 0x00]);
    // Load into RDX so the assertion depends on the guest memory round trip.
    code[27..32].copy_from_slice(&[0x48, 0x8b, 0x54, 0x24, 0x20]);
    // Add rsp, 0x28
    code[32..36].copy_from_slice(&[0x48, 0x83, 0xc4, 0x28]);
    // HLT
    code[36] = 0xf4;

    // Relocation #1 at RVA 0x1028 (offset 40..48):
    code[40..48].copy_from_slice(&(pref_base + 0x1050).to_le_bytes());
    // Relocation #2 at RVA 0x1030 (offset 48..56):
    code[48..56].copy_from_slice(&(pref_base + 0x2000).to_le_bytes());

    // TLS Callbacks in .text:
    // Callback 1 at RVA 0x1060: increments RAX by 1, ret (48 ff c0 c3 -> inc rax / ret)
    code[0x60..0x64].copy_from_slice(&[0x48, 0xff, 0xc0, 0xc3]);
    // Callback 2 doubles RAX and records it below its stack frame.
    code[0x70..0x79].copy_from_slice(&[0x48, 0x01, 0xc0, 0x48, 0x89, 0x44, 0x24, 0xf0, 0xc3]);

    let pe_bytes = TestPeBuilder::new(pref_base, 0x1000)
        .code(&code)
        .add_dir64_reloc(0x1028) // Relocation #1
        .add_padding_reloc(0x1028) // Padding absolute reloc
        .add_dir64_reloc(0x1030) // Relocation #2
        .add_tls_callback(pref_base + 0x1060)
        .add_tls_callback(pref_base + 0x1070)
        .add_import("kernel32.dll", "GetSystemTimeAsFileTime")
        .build();

    let stack_top = 0x500000000;
    let layout = || StartupLayout {
        image_base: loaded_base,
        stack_top,
        tls_base: 0x300000000,
    };
    let registry = || {
        let mut imports = ImportRegistry::new();
        let clock = Arc::new(MockClock::new(133_000_000_000_000_000));
        register_standard_services(&mut imports, Arc::new(ServicesState::new(clock)));
        imports
    };
    let probe = xenora_loader::probe_pe_bytes(&pe_bytes, &Default::default()).unwrap();
    assert_eq!(probe.imports.len(), 1);
    assert_eq!(probe.imports[0].symbol, "GetSystemTimeAsFileTime");
    assert!(probe.imports[0].is_supported);

    let startup =
        run_synthetic_startup(&pe_bytes, layout(), registry(), 1000).expect("run fixture startup");
    assert_eq!(startup.relocations.applied_dir64, 2);
    assert_eq!(startup.relocations.skipped_absolute, 1);
    assert_eq!(startup.relocations.delta, 0x100000000);
    assert_eq!(startup.bound_imports, 1);
    assert_eq!(startup.tls_callbacks, 2);
    assert_eq!(startup.execution.exit_reason, ExitReason::Halt);
    assert_eq!(
        startup.execution.final_state.get(Register::Rsp),
        stack_top - 40
    );
    assert_eq!(startup.execution.final_state.get(Register::Rdx), 0x77);

    let memory = &startup.image.memory;
    assert_eq!(
        memory.read_u64_bypass(loaded_base + 0x1028).unwrap(),
        loaded_base + 0x1050
    );
    assert_eq!(
        memory.read_u64_bypass(loaded_base + 0x1030).unwrap(),
        loaded_base + 0x2000
    );
    assert_eq!(memory.read_u64(stack_top - 40 - 16).unwrap(), 2); // (0 + 1) * 2
    assert_eq!(
        memory.read_u64(stack_top - 40 - 0x28 + 0x10).unwrap(),
        133_000_000_000_000_000
    );

    let mut halting_tls = pe_bytes;
    let callback_offset = startup.image.pe.rva_to_file_offset(0x1060).unwrap();
    halting_tls[callback_offset] = 0xf4;
    let missing = run_synthetic_startup(&halting_tls, layout(), ImportRegistry::new(), 1000)
        .err()
        .expect("import binding must fail before the halting callback");
    assert_eq!(
        missing.downcast_ref::<xenora_loader::ImportError>(),
        Some(&xenora_loader::ImportError::UnresolvedImport {
            module: "kernel32.dll".into(),
            symbol: "GetSystemTimeAsFileTime".into(),
        }),
    );
    let fault = run_synthetic_startup(&halting_tls, layout(), registry(), 1000)
        .err()
        .expect("callback halt must fail startup");
    assert!(matches!(
        fault.downcast_ref::<TlsError>(),
        Some(TlsError::CallbackExecutionFault(ExitReason::Halt))
    ));
}

#[test]
fn test_fixture_b_xmem_guest_allocator() {
    // Tests Xbox Scarlett memory allocation service (xmem.dll!XMemAlloc):
    // 1. Invokes XMemAlloc via synthetic IAT call.
    // 2. Writes data to returned guest address.
    // 3. Reads data back and verifies integrity.

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

fn run_fixture_c(handler: xenora_loader::imports::ServiceHandler) {
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

    // Place an original full-screen triangle in the guest image.
    let positions: [f32; 12] = [-1., -1., 0.5, 1., 3., -1., 0.5, 1., -1., 3., 0.5, 1.];
    for (slot, value) in code[0x40..0x70]
        .as_chunks_mut::<4>()
        .0
        .iter_mut()
        .zip(positions)
    {
        slot.copy_from_slice(&value.to_le_bytes());
    }
    let pe_bytes = TestPeBuilder::new(pref_base, 0x1000)
        .code(&code)
        .add_import("xenora_fixture.dll", "RenderFrame")
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
    import_registry.register("xenora_fixture.dll", "RenderFrame", handler);

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
    cpu.set(Register::Rcx, pref_base + 0x1040);
    let result = dbt.run(&mut cpu, &mut memory, 1000);

    assert_eq!(result.exit_reason, ExitReason::Halt);
    assert_eq!(result.final_state.get(Register::Rsp), stack_top);
    assert_eq!(result.final_state.get(Register::Rax), 0x42);
    assert_eq!(dbt.metrics.service_calls, 1);
    assert_eq!(dbt.metrics.interpreter_fallbacks, 0);
}

#[test]
fn test_fixture_c_mock_render_service_dispatch() {
    run_fixture_c(Box::new(|args, _, mem| {
        assert_eq!(
            mem.read_u64(args.get(0, mem).map_err(ExitReason::MemoryFault)?)
                .map_err(ExitReason::MemoryFault)?,
            0xbf80_0000_bf80_0000
        );
        Ok(0x42)
    }));
}

#[test]
#[cfg_attr(
    not(feature = "gpu-tests"),
    ignore = "needs native Metal; run scripts/test-gpu.sh"
)]
fn test_fixture_c_service_to_metal_framebuffer() {
    use xenora_metal::NativeCompute;
    use xenora_shader::{Architecture, PIXEL_ENTRY_POINT, VERTEX_ENTRY_POINT, compile_graphics};

    run_fixture_c(Box::new(|args, _, mem| {
        let mut bytes = [0; 48];
        let address = args.get(0, mem).map_err(ExitReason::MemoryFault)?;
        mem.read_bytes(address, &mut bytes)
            .map_err(ExitReason::MemoryFault)?;
        let mut vertices = [[0.0; 4]; 3];
        for (value, bytes) in vertices.iter_mut().flatten().zip(bytes.as_chunks::<4>().0) {
            *value = f32::from_le_bytes(*bytes);
        }
        let shader = compile_graphics(
            Architecture::Gfx1030,
            include_bytes!("../../../fixtures/rdna2/x9-constant-vs.bin"),
            include_bytes!("../../../fixtures/rdna2/x9-constant-ps.bin"),
        )
        .expect("compile original graphics fixture");
        let pixels = NativeCompute::new()
            .expect("native Metal device")
            .render_rgba8(
                &shader.msl,
                [VERTEX_ENTRY_POINT, PIXEL_ENTRY_POINT],
                &vertices,
                &[[0.0; 4]; 3],
                [4, 3],
                [19, 43, 93, 251],
            )
            .expect("Metal draw and readback");
        // The authored triangle covers every pixel and its shader exports opaque red.
        assert_eq!(pixels, [255, 0, 0, 255].repeat(12));
        Ok(0x42)
    }));
}

const PROCESS_FIXTURE_STACK_TOP: u64 = 0x500000000;
const PROCESS_FIXTURE_OUTPUT: u64 = PROCESS_FIXTURE_STACK_TOP - 0x100;

fn call_process_fixture(
    state: &Arc<ServicesState>,
    module: &str,
    symbol: &str,
    rcx: u64,
    rdx: u64,
) -> xenora_loader::startup::StartupResult {
    call_identity_fixture(state, module, symbol, [rcx, rdx, 0])
}

fn call_identity_fixture(
    state: &Arc<ServicesState>,
    module: &str,
    symbol: &str,
    args: [u64; 3],
) -> xenora_loader::startup::StartupResult {
    call_service_fixture(state, module, symbol, &args)
}

fn call_service_fixture(
    state: &Arc<ServicesState>,
    module: &str,
    symbol: &str,
    args: &[u64],
) -> xenora_loader::startup::StartupResult {
    let image_base = 0x140000000;
    assert!(args.len() <= 7);
    let mut code = vec![0x48, 0x83, 0xec, 0x48]; // home space, stack arguments and alignment
    let registers = [[0x48, 0xb9], [0x48, 0xba], [0x49, 0xb8], [0x49, 0xb9]];
    for (register, value) in registers.into_iter().zip(args) {
        code.extend_from_slice(&register);
        code.extend_from_slice(&value.to_le_bytes());
    }
    for (index, value) in args.iter().skip(registers.len()).enumerate() {
        code.extend_from_slice(&[0x48, 0xb8]); // mov rax, imm64
        code.extend_from_slice(&value.to_le_bytes());
        // mov [rsp + 32 + index * 8], rax (before the call pushes its return address)
        code.extend_from_slice(&[0x48, 0x89, 0x44, 0x24, 0x20 + index as u8 * 8]);
    }
    code.extend_from_slice(&[0x48, 0xb8]); // mov rax, address of output slot
    code.extend_from_slice(&PROCESS_FIXTURE_OUTPUT.to_le_bytes());
    // mov qword ptr [rax], -1, seeding the BOOL and adjacent bytes
    code.extend_from_slice(&[0x48, 0xc7, 0x00, 0xff, 0xff, 0xff, 0xff]);
    append_import_call(&mut code, 0x2060);
    code.extend_from_slice(&[0x48, 0x83, 0xc4, 0x48, 0xf4]); // release frame and halt

    let bytes = TestPeBuilder::new(image_base, 0x1000)
        .code(&code)
        .add_import(module, symbol)
        .build();
    let mut imports = ImportRegistry::new();
    register_standard_services(&mut imports, Arc::clone(state));
    let result = run_synthetic_startup(
        &bytes,
        StartupLayout {
            image_base,
            stack_top: PROCESS_FIXTURE_STACK_TOP,
            tls_base: 0x300000000,
        },
        imports,
        100,
    )
    .expect("start process-service PE fixture");
    assert_eq!(result.bound_imports, 1);
    assert_eq!(result.execution.exit_reason, ExitReason::Halt);
    result
}

#[test]
fn test_process_priority_boost_setter_through_iat() {
    let state = Arc::new(ServicesState::new(Arc::new(MockClock::new(0))));
    let set = |module, handle, disable| {
        call_process_fixture(&state, module, "SetProcessPriorityBoost", handle, disable)
            .execution
            .final_state
            .get(Register::Rax)
    };
    let last_error = || {
        call_process_fixture(&state, "kernel32.dll", "GetLastError", 0, 0)
            .execution
            .final_state
            .get(Register::Rax)
    };
    let api_set = "API-MS-WIN-CORE-PROCESSTHREADS-L1-1-2.DLL";
    assert!(!state.priority_boost_disabled());
    assert_eq!(set(api_set, u64::MAX, 2), 1);
    assert!(state.priority_boost_disabled());
    assert_eq!(set(api_set, u64::MAX, 2), 1);
    assert!(state.priority_boost_disabled());
    for invalid in [0, 0x4100, u64::MAX - 1] {
        assert_eq!(set(api_set, invalid, 0), 0);
        assert_eq!(last_error(), 6);
        assert!(state.priority_boost_disabled());
    }
    // BOOL occupies the low 32 bits of the argument register.
    assert_eq!(set("kernel32.dll", u64::MAX, 1 << 32), 1);
    assert!(!state.priority_boost_disabled());
    assert_eq!(last_error(), 6);
}

#[test]
fn test_current_process_identity_through_iat() {
    let mut state = ServicesState::new(Arc::new(MockClock::new(0)));
    state.process_id = 12345;
    let state = Arc::new(state);
    let api_set = "api-ms-win-core-processthreads-l1-1-0.dll";
    let call = |module, symbol, handle| {
        call_process_fixture(&state, module, symbol, handle, 0)
            .execution
            .final_state
            .get(Register::Rax)
    };
    let handle = call(api_set, "GetCurrentProcess", 0);
    assert_eq!(handle, u64::MAX);
    assert_eq!(call("kernel32.dll", "GetCurrentProcess", 0), handle);
    assert_eq!(call(api_set, "GetProcessId", handle), 12345);
    assert_eq!(call(api_set, "GetProcessId", 12345), 0);
    assert_eq!(
        call(
            "api-ms-win-core-errorhandling-l1-1-0.dll",
            "GetLastError",
            0
        ),
        6
    );
    assert_eq!(call("kernel32.dll", "GetProcessId", handle), 12345);
}

#[test]
fn test_process_priority_boost_query_through_iat() {
    let state = Arc::new(ServicesState::new(Arc::new(MockClock::new(0))));
    let api_set = "api-ms-win-core-processthreads-l1-1-2.dll";
    let query =
        |handle, ptr| call_process_fixture(&state, api_set, "GetProcessPriorityBoost", handle, ptr);
    let read_output = |result: &xenora_loader::startup::StartupResult| {
        result
            .image
            .memory
            .read_u64(PROCESS_FIXTURE_OUTPUT)
            .unwrap()
    };
    for (disable, expected) in [(false, 0), (true, 1), (true, 1), (false, 0)] {
        let setter = call_process_fixture(
            &state,
            api_set,
            "SetProcessPriorityBoost",
            u64::MAX,
            u64::from(disable),
        );
        assert_eq!(setter.execution.final_state.get(Register::Rax), 1);
        let result = query(u64::MAX, PROCESS_FIXTURE_OUTPUT);
        assert_eq!(result.execution.final_state.get(Register::Rax), 1);
        assert_eq!(read_output(&result), 0xffff_ffff_0000_0000 | expected);
    }

    for (handle, ptr, error) in [
        (0, PROCESS_FIXTURE_OUTPUT, 6),
        (u64::MAX, 0, 998),
        (u64::MAX, 0x140001000, 998), // Executable section has no write permission.
        (u64::MAX, PROCESS_FIXTURE_STACK_TOP - 2, 998),
    ] {
        let result = query(handle, ptr);
        assert_eq!(result.execution.final_state.get(Register::Rax), 0);
        assert_eq!(read_output(&result), u64::MAX);
        let result = call_process_fixture(&state, "kernel32.dll", "GetLastError", 0, 0);
        assert_eq!(result.execution.final_state.get(Register::Rax), error);
    }
}

#[test]
fn test_guest_performance_counter_through_iat() {
    let clock = Arc::new(MockClock::new(133_000_000_000_000_000));
    let state = Arc::new(ServicesState::new(clock.clone()));
    let query = |module, symbol| {
        let result = call_process_fixture(&state, module, symbol, PROCESS_FIXTURE_OUTPUT, 0);
        assert_eq!(result.execution.final_state.get(Register::Rax), 1);
        result
            .image
            .memory
            .read_u64(PROCESS_FIXTURE_OUTPUT)
            .unwrap()
    };
    let kernel = "kernel32.dll";
    let profile = "api-ms-win-core-profile-l1-1-0.dll";
    assert_eq!(query(kernel, "QueryPerformanceCounter"), 0);
    assert_eq!(query(profile, "QueryPerformanceFrequency"), 10_000_000);
    clock.advance(2_500_000);
    assert_eq!(query(profile, "QueryPerformanceCounter"), 2_500_000);
    assert_eq!(query(kernel, "QueryPerformanceFrequency"), 10_000_000);

    for symbol in ["QueryPerformanceCounter", "QueryPerformanceFrequency"] {
        for ptr in [0, 0x140001000, PROCESS_FIXTURE_STACK_TOP - 2] {
            let result = call_process_fixture(&state, profile, symbol, ptr, 0);
            assert_eq!(result.execution.final_state.get(Register::Rax), 0);
            assert_eq!(
                result
                    .image
                    .memory
                    .read_u64(PROCESS_FIXTURE_OUTPUT)
                    .unwrap(),
                u64::MAX
            );
            let error = call_process_fixture(&state, kernel, "GetLastError", 0, 0);
            assert_eq!(error.execution.final_state.get(Register::Rax), 998);
        }
    }
    clock.advance(u64::MAX);
    assert_eq!(query(kernel, "QueryPerformanceCounter"), i64::MAX as u64);
    let error = call_process_fixture(&state, kernel, "GetLastError", 0, 0);
    assert_eq!(error.execution.final_state.get(Register::Rax), 998);
}

#[test]
fn test_guest_performance_counter_with_host_clock() {
    let state = Arc::new(ServicesState::new(Arc::new(
        xenora_loader::services::SystemClock,
    )));
    let read_counter = || {
        let result = call_process_fixture(
            &state,
            "kernel32.dll",
            "QueryPerformanceCounter",
            PROCESS_FIXTURE_OUTPUT,
            0,
        );
        assert_eq!(result.execution.final_state.get(Register::Rax), 1);
        result
            .image
            .memory
            .read_u64(PROCESS_FIXTURE_OUTPUT)
            .unwrap()
    };

    let first = read_counter();
    std::thread::sleep(std::time::Duration::from_millis(1));
    assert!(read_counter() > first);
}

#[test]
fn test_guest_uptime_wraps_only_at_32_bits() {
    let clock = Arc::new(MockClock::new(133_000_000_000_000_000));
    clock.advance(5_000_000);
    let state = Arc::new(ServicesState::new(clock.clone()));
    let query = |module, symbol| {
        call_service_fixture(&state, module, symbol, &[])
            .execution
            .final_state
            .get(Register::Rax)
    };
    let kernel = "kernel32.dll";
    let sysinfo = "api-ms-win-core-sysinfo-l1-1-0.dll";
    assert_eq!(query(kernel, "GetTickCount"), 0);
    assert_eq!(query(sysinfo, "GetTickCount64"), 0);
    clock.advance(9_999);
    assert_eq!(query(kernel, "GetTickCount"), 0);
    clock.advance(1);
    assert_eq!(query(sysinfo, "GetTickCount64"), 1);

    let max = u64::from(u32::MAX);
    clock.advance((max - 1) * 10_000);
    assert_eq!(query(sysinfo, "GetTickCount"), max);
    assert_eq!(query(kernel, "GetTickCount64"), max);
    clock.advance(10_000);
    assert_eq!(query(sysinfo, "GetTickCount"), 0);
    assert_eq!(query(kernel, "GetTickCount64"), max + 1);
}

fn append_import_call(code: &mut Vec<u8>, slot_rva: i32) {
    // call qword ptr [rip + disp32], through the fixture's import slot.
    let next_rva = 0x1000 + code.len() as i32 + 6;
    code.extend_from_slice(&[0xff, 0x15]);
    code.extend_from_slice(&(slot_rva - next_rva).to_le_bytes());
}

#[test]
fn test_guest_thread_handle_lifecycle_through_iat() {
    let state = Arc::new(ServicesState::new(Arc::new(MockClock::new(0))));
    let api = "api-ms-win-core-processthreads-l1-1-0.dll";
    let handle_api = "api-ms-win-core-handle-l1-1-0.dll";
    let call = |module, symbol, args| {
        call_identity_fixture(&state, module, symbol, args)
            .execution
            .final_state
            .get(Register::Rax)
    };
    let last_error = || call("kernel32.dll", "GetLastError", [0; 3]);
    let id = call(api, "GetCurrentThreadId", [0; 3]);
    let pseudo = call(api, "GetCurrentThread", [0; 3]);
    let thread_id = |handle| call(api, "GetThreadId", [handle, 0, 0]);
    assert_eq!(pseudo, u64::MAX - 1);

    const QUERY_LIMITED: u64 = 0x0800;
    const SET_LIMITED: u64 = 0x0400;
    let open = |rights, inherit, id| call(api, "OpenThread", [rights, inherit, id]);
    let query = |handle| call(api, "GetThreadPriority", [handle, 0, 0]);
    let set = |handle, priority| call(api, "SetThreadPriority", [handle, priority, 0]);
    let close = |handle| call(handle_api, "CloseHandle", [handle, 0, 0]);

    assert_eq!(open(QUERY_LIMITED, 0, id + 1), 0);
    assert_eq!(last_error(), 87);
    assert_eq!(open(QUERY_LIMITED, 1, id), 0);
    assert_eq!(last_error(), 50);
    assert_eq!(open(0x0002, 0, id), 0);
    assert_eq!(last_error(), 50);
    let query_only = open(QUERY_LIMITED, 0, id);
    let set_only = open(SET_LIMITED, 0, id);
    assert_ne!(query_only, 0);
    assert_ne!(set_only, 0);
    assert_ne!(query_only, set_only);
    assert_eq!(thread_id(pseudo), id);
    assert_eq!(thread_id(query_only), id);
    assert_eq!(thread_id(set_only), 0);
    assert_eq!(last_error(), 5);
    assert_eq!(thread_id(u64::MAX), 0);
    assert_eq!(last_error(), 6);
    assert_eq!(query(query_only), 0);
    assert_eq!(set(query_only, 1), 0);
    assert_eq!(last_error(), 5);
    assert_eq!(query(set_only), 0x7fff_ffff);
    assert_eq!(last_error(), 5);
    assert_eq!(set(set_only, u64::MAX), 1);
    assert_eq!(query(query_only), u64::from(u32::MAX)); // signed int -1
    assert_eq!(query(pseudo), u64::from(u32::MAX));
    assert_eq!(set(pseudo, 3), 0);
    assert_eq!(last_error(), 87);
    assert_eq!(set(pseudo, 0x0001_0000), 0);
    assert_eq!(last_error(), 50);
    assert_eq!(query(pseudo), u64::from(u32::MAX));
    assert_eq!(close(pseudo), 1);
    assert_eq!(close(query_only), 1);
    assert_eq!(thread_id(query_only), 0);
    assert_eq!(last_error(), 6);
    assert_eq!(query(query_only), 0x7fff_ffff);
    assert_eq!(last_error(), 6);
    assert_eq!(close(query_only), 0);
    assert_eq!(last_error(), 6);
    let reopened = open(QUERY_LIMITED, 0, id);
    assert_ne!(reopened, query_only);
    assert_eq!(thread_id(reopened), id);
    assert_eq!(thread_id(query_only), 0);
    assert_eq!(query(reopened), u64::from(u32::MAX));
    assert_eq!(last_error(), 6);
}

#[test]
fn test_guest_duplicates_thread_handle_through_iat() {
    let state = Arc::new(ServicesState::new(Arc::new(MockClock::new(0))));
    let call = |symbol, arg| {
        call_process_fixture(&state, "kernel32.dll", symbol, arg, 0)
            .execution
            .final_state
            .get(Register::Rax)
    };
    let duplicate = |source, output, rights, inherit, options| {
        let result = call_service_fixture(
            &state,
            "api-ms-win-core-handle-l1-1-0.dll",
            "DuplicateHandle",
            &[u64::MAX, source, u64::MAX, output, rights, inherit, options],
        );
        let success = result.execution.final_state.get(Register::Rax);
        let handle = result
            .image
            .memory
            .read_u64(PROCESS_FIXTURE_OUTPUT)
            .unwrap();
        (success, handle)
    };

    let id = call("GetCurrentThreadId", 0);
    let pseudo = call("GetCurrentThread", 0);
    let (success, source) = duplicate(pseudo, PROCESS_FIXTURE_OUTPUT, 0x0800, 0, 0);
    assert_eq!(success, 1);
    assert_ne!(source, pseudo);
    assert_eq!(call("GetThreadId", source), id);

    assert_eq!(
        duplicate(source, PROCESS_FIXTURE_OUTPUT, 0x0400, 0, 0),
        (0, u64::MAX)
    );
    assert_eq!(call("GetLastError", 0), 5);
    assert_eq!(duplicate(source, 0, 0, 0, 2), (0, u64::MAX));
    assert_eq!(call("GetLastError", 0), 998);
    assert_eq!(
        duplicate(source, PROCESS_FIXTURE_OUTPUT, 0, 1, 2),
        (0, u64::MAX)
    );
    assert_eq!(call("GetLastError", 0), 50);
    assert_eq!(
        duplicate(source, PROCESS_FIXTURE_OUTPUT, 0, 0, 1),
        (0, u64::MAX)
    );
    assert_eq!(call("GetLastError", 0), 50);
    assert_eq!(call("GetThreadId", source), id);

    let (success, copy) = duplicate(source, PROCESS_FIXTURE_OUTPUT, u64::MAX, 0, 2);
    assert_eq!(success, 1);
    assert_ne!(source, copy);
    assert_eq!(call("CloseHandle", source), 1);
    assert_eq!(call("GetThreadId", copy), id);
    assert_eq!(
        duplicate(source, PROCESS_FIXTURE_OUTPUT, 0, 0, 2),
        (0, u64::MAX)
    );
    assert_eq!(call("GetLastError", 0), 6);
    assert_eq!(call("CloseHandle", copy), 1);
    assert_eq!(call("GetThreadId", copy), 0);
}

#[test]
fn test_guest_crt_memory_through_iat() {
    let state = Arc::new(ServicesState::new(Arc::new(MockClock::new(0))));
    let slot = PROCESS_FIXTURE_OUTPUT;
    let call = |symbol, args: &[u64]| {
        call_service_fixture(&state, "api-ms-win-crt-string-l1-1-0.dll", symbol, args)
    };
    let fill = call("memset", &[slot + 2, 0x141, 3]);
    assert_eq!(fill.execution.final_state.get(Register::Rax), slot + 2);
    assert_eq!(
        fill.image.memory.read_u64(slot).unwrap().to_le_bytes(),
        [255, 255, 65, 65, 65, 255, 255, 255]
    );

    let copy = call("memcpy", &[slot + 8, slot, 8]);
    assert_eq!(copy.execution.final_state.get(Register::Rax), slot + 8);
    assert_eq!(copy.image.memory.read_u64(slot + 8).unwrap(), u64::MAX);

    let shifted = call("memmove", &[slot + 1, slot, 8]);
    assert_eq!(shifted.image.memory.read_u64(slot + 1).unwrap(), u64::MAX);
    let shifted = call("memmove", &[slot, slot + 1, 8]);
    assert_eq!(
        shifted.image.memory.read_u64(slot).unwrap().to_le_bytes(),
        [255, 255, 255, 255, 255, 255, 255, 0]
    );

    for (a, b, expected) in [
        (slot, slot + 8, 255),
        (slot + 8, slot, -255),
        (slot, slot, 0),
    ] {
        let result = call("memcmp", &[a, b, 8]);
        assert_eq!(
            result.execution.final_state.get(Register::Rax) as i32,
            expected
        );
    }
}

#[test]
fn test_guest_crt_string_length_through_iat() {
    let state = Arc::new(ServicesState::new(Arc::new(MockClock::new(0))));
    for module in ["ucrtbase.dll", "api-ms-win-crt-string-l1-1-0.dll"] {
        // The fixture seeds the output slot with eight 0xff bytes followed by zeroed stack bytes.
        for (symbol, address, bound, expected) in [
            ("strlen", PROCESS_FIXTURE_OUTPUT, 0, 8),
            ("strlen", PROCESS_FIXTURE_OUTPUT + 8, 0, 0),
            ("strnlen", PROCESS_FIXTURE_OUTPUT, 3, 3),
            ("strnlen", PROCESS_FIXTURE_OUTPUT, 12, 8),
            ("strnlen", PROCESS_FIXTURE_STACK_TOP, 0, 0),
        ] {
            let result = call_service_fixture(&state, module, symbol, &[address, bound]);
            assert_eq!(result.execution.final_state.get(Register::Rax), expected);
        }
    }
}

#[test]
fn test_guest_event_signaling_through_iat() {
    let state = Arc::new(ServicesState::new(Arc::new(MockClock::new(0))));
    let call = |module, symbol, args: &[u64]| {
        call_service_fixture(&state, module, symbol, args)
            .execution
            .final_state
            .get(Register::Rax)
    };
    let kernel = "kernel32.dll";
    let synch = "api-ms-win-core-synch-l1-1-0.dll";
    let wait = |handle, timeout| call(synch, "WaitForSingleObject", &[handle, timeout]);
    let error = || call(kernel, "GetLastError", &[]);

    let manual = call(synch, "CreateEventW", &[0, 1, 0, 0]);
    let auto = call(kernel, "CreateEventA", &[0, 0, 1, 0]);
    assert_ne!(manual, 0);
    assert_ne!(auto, 0);
    assert_ne!(manual, auto);
    assert_eq!(wait(manual, 0), 258);
    assert_eq!(wait(manual, 1), u32::MAX as u64);
    assert_eq!(error(), 50);
    assert_eq!(call(synch, "SetEvent", &[manual]), 1);
    assert_eq!(wait(manual, u32::MAX as u64), 0);
    assert_eq!(wait(manual, 0), 0);
    assert_eq!(call(synch, "ResetEvent", &[manual]), 1);
    assert_eq!(wait(manual, 0), 258);
    assert_eq!(wait(auto, 0), 0);
    assert_eq!(wait(auto, 0), 258);
    assert_eq!(call(synch, "SetEvent", &[auto]), 1);
    assert_eq!(call(synch, "SetEvent", &[auto]), 1);
    assert_eq!(wait(auto, 0), 0);
    assert_eq!(wait(auto, 0), 258);

    assert_eq!(call(kernel, "GetThreadId", &[auto]), 0);
    assert_eq!(error(), 6);
    let thread = call(kernel, "GetCurrentThread", &[]);
    assert_eq!(call(synch, "SetEvent", &[thread]), 0);
    assert_eq!(error(), 6);
    assert_eq!(call(kernel, "CloseHandle", &[manual]), 1);
    assert_eq!(wait(manual, 0), u32::MAX as u64);
    assert_eq!(error(), 6);
    assert_eq!(call(kernel, "CreateEventW", &[0, 0, 0, 1]), 0);
    assert_eq!(error(), 50);
    let copy = call_service_fixture(
        &state,
        kernel,
        "DuplicateHandle",
        &[
            u64::MAX,
            auto,
            u64::MAX,
            PROCESS_FIXTURE_OUTPUT,
            0x10_0000,
            0,
            0,
        ],
    );
    assert_eq!(copy.execution.final_state.get(Register::Rax), 1);
    let copy = copy.image.memory.read_u64(PROCESS_FIXTURE_OUTPUT).unwrap();
    assert_eq!(call(synch, "SetEvent", &[copy]), 0);
    assert_eq!(error(), 5);
    assert_eq!(call(synch, "SetEvent", &[auto]), 1);
    assert_eq!(wait(copy, 0), 0);
    assert_eq!(wait(auto, 0), 258);
    assert_eq!(call(synch, "SetEvent", &[auto]), 1);
    assert_eq!(call(kernel, "CloseHandle", &[auto]), 1);
    assert_eq!(wait(copy, 0), 0);
    assert_eq!(call(kernel, "CloseHandle", &[copy]), 1);
}

#[test]
fn test_virtual_allocation_release_through_iat() {
    const IMAGE: u64 = 0x140000000;
    const HEAP: u64 = 0x8_0000_0000;
    const MODULE: &str = "api-ms-win-core-memory-l1-1-0.dll";

    let run = |args: [u64; 4], free: Option<[u64; 3]>| {
        let mut code = vec![0x48, 0x83, 0xec, 0x28]; // Win64 home space and alignment
        let registers = [[0x48, 0xb9], [0x48, 0xba], [0x49, 0xb8], [0x49, 0xb9]];
        for (register, value) in registers.into_iter().zip(args) {
            code.extend_from_slice(&register);
            code.extend_from_slice(&value.to_le_bytes());
        }
        append_import_call(&mut code, 0x2060);
        let mut fixture = TestPeBuilder::new(IMAGE, 0x1000).add_import(MODULE, "VirtualAlloc");
        if let Some(free) = free {
            for (register, value) in registers.into_iter().zip(free) {
                code.extend_from_slice(&register);
                code.extend_from_slice(&value.to_le_bytes());
            }
            append_import_call(&mut code, 0x21a0);
            fixture = fixture.add_import(MODULE, "VirtualFree");
        }
        code.extend_from_slice(&[0x48, 0x83, 0xc4, 0x28, 0xf4]);
        let state = Arc::new(ServicesState::new(Arc::new(MockClock::new(0))));
        let mut imports = ImportRegistry::new();
        register_standard_services(&mut imports, Arc::clone(&state));
        let result = run_synthetic_startup(
            &fixture.code(&code).build(),
            StartupLayout {
                image_base: IMAGE,
                stack_top: PROCESS_FIXTURE_STACK_TOP,
                tls_base: 0x300000000,
            },
            imports,
            100,
        )
        .expect("start memory-service PE fixture");
        assert_eq!(result.execution.exit_reason, ExitReason::Halt);
        assert_eq!(result.bound_imports, if free.is_some() { 2 } else { 1 });
        let value = result.execution.final_state.get(Register::Rax);
        let error = state.last_error.load(std::sync::atomic::Ordering::SeqCst);
        (result.image.memory, value, error)
    };

    let (mut memory, value, error) = run([HEAP + 0x800, 0x1000, 0x3000, 4], None);
    assert_eq!((value, error), (HEAP, 0));
    assert_eq!(memory.read_u32(HEAP + 0x1ffc).unwrap(), 0);
    memory.write_u32(HEAP, 123).unwrap();
    assert_eq!(memory.read_u32(HEAP).unwrap(), 123);
    assert!(memory.read_u32(HEAP + 0x2000).is_err());

    let (memory, value, error) = run([HEAP, 1, 0x3000, 4], Some([HEAP, 0, 0x8000]));
    assert_eq!((value, error), (1, 0));
    assert!(memory.read_u32(HEAP).is_err());

    for (free_args, expected_error) in [
        ([HEAP + 1, 0, 0x8000], 487),
        ([HEAP, 1, 0x8000], 87),
        ([HEAP, 0, 0x4000], 50),
        ([IMAGE + 0x1000, 0, 0x8000], 487),
        ([PROCESS_FIXTURE_STACK_TOP - 0x1_0000, 0, 0x8000], 487),
    ] {
        let (memory, value, error) = run([HEAP, 1, 0x3000, 4], Some(free_args));
        assert_eq!((value, error), (0, expected_error));
        assert_eq!(memory.read_u32(HEAP).unwrap(), 0);
        assert!(memory.fetch_instruction_byte(IMAGE + 0x1000).is_ok());
    }
    let (mut memory, address, error) = run([0, 1, 0x3000, 2], None);
    assert_ne!(address, 0);
    assert_eq!(address % 65_536, 0);
    assert_eq!(error, 0);
    assert_eq!(memory.read_u32(address).unwrap(), 0);
    assert!(memory.write_u32(address, 1).is_err());

    let (memory, address, error) = run([0, 1, 0x3000, 1], None);
    assert_ne!(address, 0);
    assert_eq!(error, 0);
    assert!(memory.read_u32(address).is_err());

    for (args, expected_error) in [
        ([0, 0, 0x3000, 4], 87),
        ([0, u64::MAX, 0x3000, 4], 87),
        ([0, 65 * 1024 * 1024, 0x3000, 4], 8),
        ([0, 0x1000, 0x2000, 4], 50),
        ([0, 0x1000, 0x3000, 0x40], 87),
        ([IMAGE + 0x1000, 1, 0x3000, 4], 487),
    ] {
        let (_, value, error) = run(args, None);
        assert_eq!((value, error), (0, expected_error));
    }
}
