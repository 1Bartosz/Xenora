// SPDX-License-Identifier: GPL-3.0-or-later
#![no_main]

use libfuzzer_sys::fuzz_target;
use std::collections::BTreeSet;
use xenora_loader::probe::{ProbeConfig, probe_pe_bytes};

const MAX_CODE_BYTES: usize = 4096;
const CONFIG: ProbeConfig = ProbeConfig {
    max_disasm_bytes: MAX_CODE_BYTES,
    max_instructions: 2048,
    max_worklist_entries: 1024,
};

const PE_OFFSET: usize = 0x40;
const COFF: usize = PE_OFFSET + 4;
const OPTIONAL: usize = COFF + 20;
const OPTIONAL_LEN: u16 = 0xf0;
const TEXT: usize = OPTIONAL + OPTIONAL_LEN as usize;
const HEADER_LEN: usize = 0x200;
const TEXT_RVA: u32 = 0x1000;
const SECTION_ALIGNMENT: u32 = 0x1000;

const HEADER: [u8; HEADER_LEN] = {
    let mut header = [0; HEADER_LEN];
    write_field(&mut header, 0, b"MZ");
    write_field(&mut header, 0x3c, &(PE_OFFSET as u32).to_le_bytes());
    write_field(&mut header, PE_OFFSET, b"PE\0\0");
    write_field(&mut header, COFF, &0x8664u16.to_le_bytes()); // Machine
    write_field(&mut header, COFF + 2, &1u16.to_le_bytes()); // NumberOfSections
    write_field(&mut header, COFF + 16, &OPTIONAL_LEN.to_le_bytes());
    write_field(&mut header, OPTIONAL, &0x20bu16.to_le_bytes()); // Magic
    write_field(&mut header, OPTIONAL + 16, &TEXT_RVA.to_le_bytes()); // AddressOfEntryPoint
    write_field(&mut header, OPTIONAL + 20, &TEXT_RVA.to_le_bytes()); // BaseOfCode
    write_field(&mut header, OPTIONAL + 24, &0x1_4000_0000u64.to_le_bytes()); // ImageBase
    write_field(&mut header, OPTIONAL + 32, &SECTION_ALIGNMENT.to_le_bytes());
    write_field(&mut header, OPTIONAL + 36, &0x200u32.to_le_bytes()); // FileAlignment
    write_field(
        &mut header,
        OPTIONAL + 60,
        &(HEADER_LEN as u32).to_le_bytes(),
    ); // SizeOfHeaders
    write_field(&mut header, OPTIONAL + 108, &16u32.to_le_bytes()); // NumberOfRvaAndSizes
    write_field(&mut header, TEXT, b".text");
    write_field(&mut header, TEXT + 12, &TEXT_RVA.to_le_bytes()); // VirtualAddress
    write_field(&mut header, TEXT + 20, &(HEADER_LEN as u32).to_le_bytes()); // PointerToRawData
    write_field(&mut header, TEXT + 36, &0x6000_0020u32.to_le_bytes()); // CODE | EXECUTE | READ
    header
};

const fn write_field(header: &mut [u8], offset: usize, bytes: &[u8]) {
    let (_, tail) = header.split_at_mut(offset);
    let (field, _) = tail.split_at_mut(bytes.len());
    field.copy_from_slice(bytes);
}

fn build_image(payload: &[u8]) -> Vec<u8> {
    let mut header = HEADER;
    let code_size = payload.len() as u32;
    let image_size = TEXT_RVA + code_size.next_multiple_of(SECTION_ALIGNMENT);
    write_field(&mut header, OPTIONAL + 4, &code_size.to_le_bytes()); // SizeOfCode
    write_field(&mut header, OPTIONAL + 56, &image_size.to_le_bytes()); // SizeOfImage
    write_field(&mut header, TEXT + 8, &code_size.to_le_bytes()); // VirtualSize
    write_field(&mut header, TEXT + 16, &code_size.to_le_bytes()); // SizeOfRawData

    // The probe decodes up to EOF, so leave the payload unpadded.
    let mut image = Vec::with_capacity(HEADER_LEN + payload.len());
    image.extend_from_slice(&header);
    image.extend_from_slice(payload);
    image
}

fuzz_target!(init: {
    let image = build_image(&[0x90, 0xc3]);
    let report = probe_pe_bytes(&image, &CONFIG).expect("startup PE probe succeeds");
    let records = report.instruction_records;
    assert!(records.len() == 2, "startup decodes two instructions");
    assert!(records[0].rva == TEXT_RVA, "startup decodes entry RVA");
    assert!(records[1].rva == TEXT_RVA + 1, "startup decodes next RVA");
}, |data: &[u8]| {
    let payload = &data[..data.len().min(MAX_CODE_BYTES)];
    let image = build_image(payload);
    let a = probe_pe_bytes(&image, &CONFIG);
    let b = probe_pe_bytes(&image, &CONFIG);
    assert!(format!("{a:?}") == format!("{b:?}"), "probe is deterministic");

    if let Ok(report) = a {
        let records = report.instruction_records;
        assert!(records.len() <= CONFIG.max_instructions, "instruction budget");
        let code_range = TEXT_RVA..TEXT_RVA + payload.len() as u32;
        let mut rvas = BTreeSet::new();
        for record in records {
            let len = record.bytes.len();
            assert!((1..=15).contains(&len), "x86 instruction length");
            assert!(code_range.contains(&record.rva), "RVA within payload");
            assert!(rvas.insert(record.rva), "RVA is unique");
            let start = (record.rva - TEXT_RVA) as usize;
            let end = start + len;
            assert!(end <= payload.len(), "instruction ends within payload");
            assert!(record.bytes == payload[start..end], "instruction bytes match payload");
        }
    }
});
