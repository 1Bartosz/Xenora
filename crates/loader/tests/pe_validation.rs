// SPDX-License-Identifier: GPL-3.0-or-later

//! Malformed-input regression tests for the PE32+ parser (#79).
//!
//! Every image here is built byte by byte from the Microsoft PE/COFF
//! specification layout: DOS header with e_lfanew at 0x3C, "PE\0\0", the
//! 20-byte COFF header, the PE32+ optional header (112 fixed bytes before the
//! data directories) and 40-byte section headers.

use xenora_loader::{ParsedPe, PeError, PeSection};

const RX: u32 = 0x6000_0020; // code | execute | read
const R: u32 = 0x4000_0040; // initialized data | read

struct Sec {
    va: u32,
    vsize: u32,
    raw_ptr: u32,
    raw_size: u32,
    flags: u32,
}

/// A two-section image: .text at RVA 0x1000 (file 0x200) and .rdata at RVA 0x2000 (file 0x400).
fn valid_sections() -> Vec<Sec> {
    vec![
        Sec {
            va: 0x1000,
            vsize: 0x100,
            raw_ptr: 0x200,
            raw_size: 0x200,
            flags: RX,
        },
        Sec {
            va: 0x2000,
            vsize: 0x80,
            raw_ptr: 0x400,
            raw_size: 0x200,
            flags: R,
        },
    ]
}

fn build(opt_size: u16, entry: u32, secs: &[Sec]) -> Vec<u8> {
    let mut pe = vec![0u8; 0x600];
    pe[0..2].copy_from_slice(b"MZ");
    pe[0x3C..0x40].copy_from_slice(&0x40u32.to_le_bytes());
    pe[0x40..0x44].copy_from_slice(b"PE\0\0");
    let coff = 0x44;
    pe[coff..coff + 2].copy_from_slice(&0x8664u16.to_le_bytes());
    pe[coff + 2..coff + 4].copy_from_slice(&(secs.len() as u16).to_le_bytes());
    pe[coff + 16..coff + 18].copy_from_slice(&opt_size.to_le_bytes());
    let opt = coff + 20;
    pe[opt..opt + 2].copy_from_slice(&0x020Bu16.to_le_bytes());
    pe[opt + 16..opt + 20].copy_from_slice(&entry.to_le_bytes());
    pe[opt + 24..opt + 32].copy_from_slice(&0x1_4000_0000u64.to_le_bytes());
    pe[opt + 32..opt + 36].copy_from_slice(&0x1000u32.to_le_bytes());
    pe[opt + 36..opt + 40].copy_from_slice(&0x200u32.to_le_bytes());
    pe[opt + 56..opt + 60].copy_from_slice(&0x3000u32.to_le_bytes());
    pe[opt + 60..opt + 64].copy_from_slice(&0x200u32.to_le_bytes());
    let table = opt + usize::from(opt_size);
    for (i, s) in secs.iter().enumerate() {
        let h = table + i * 40;
        pe[h..h + 5].copy_from_slice(b".sect");
        pe[h + 8..h + 12].copy_from_slice(&s.vsize.to_le_bytes());
        pe[h + 12..h + 16].copy_from_slice(&s.va.to_le_bytes());
        pe[h + 16..h + 20].copy_from_slice(&s.raw_size.to_le_bytes());
        pe[h + 20..h + 24].copy_from_slice(&s.raw_ptr.to_le_bytes());
        pe[h + 36..h + 40].copy_from_slice(&s.flags.to_le_bytes());
    }
    pe
}

#[test]
fn valid_image_parses_maps_and_translates() {
    let bytes = build(112, 0x1010, &valid_sections());
    let pe = ParsedPe::parse(&bytes).expect("valid image");
    assert_eq!(pe.entry_point_rva, 0x1010);
    assert_eq!(pe.sections.len(), 2);
    assert_eq!(pe.rva_to_file_offset(0x1010), Some(0x210));
    assert_eq!(pe.rva_to_file_offset(0x2004), Some(0x404));
    assert_eq!(pe.rva_to_file_offset(0x3000), None);
    let loaded = pe.load_into_memory(&bytes).expect("load");
    assert_eq!(loaded.entry_point, 0x1_4000_1010);
}

#[test]
fn optional_header_with_data_directories_is_accepted() {
    let bytes = build(240, 0x1000, &valid_sections());
    assert!(ParsedPe::parse(&bytes).is_ok());
}

#[test]
fn repository_fixtures_still_parse_and_load() {
    let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
    for path in [
        "fixtures/pe/trusted-runner.exe",
        "experiments/x1/fixtures/x2.exe",
        "experiments/x1/fixtures/x3.exe",
        "experiments/x1/fixtures/x4-17.exe",
        "experiments/x1/fixtures/x4-93.exe",
        "experiments/x1/fixtures/x5.exe",
        "experiments/x1/fixtures/x6.exe",
    ] {
        let bytes = std::fs::read(format!("{root}/{path}")).expect(path);
        let pe = ParsedPe::parse(&bytes).unwrap_or_else(|e| panic!("{path}: {e}"));
        pe.load_into_memory(&bytes)
            .unwrap_or_else(|e| panic!("{path}: {e}"));
    }
}

/// Reproducer from #79: 90 bytes, SizeOfOptionalHeader = 2, file ends after the magic.
#[test]
fn truncated_optional_header_is_rejected_not_panicking() {
    let mut bytes = vec![0u8; 90];
    bytes[0..2].copy_from_slice(b"MZ");
    bytes[0x3C..0x40].copy_from_slice(&0x40u32.to_le_bytes());
    bytes[0x40..0x44].copy_from_slice(b"PE\0\0");
    bytes[0x44..0x46].copy_from_slice(&0x8664u16.to_le_bytes());
    bytes[0x54..0x56].copy_from_slice(&2u16.to_le_bytes());
    bytes[0x58..0x5A].copy_from_slice(&0x020Bu16.to_le_bytes());
    assert_eq!(ParsedPe::parse(&bytes).unwrap_err(), PeError::TooSmall);
}

#[test]
fn optional_header_shorter_than_pe32_plus_fixed_fields_is_rejected() {
    for opt_size in [24u16, 64, 111] {
        let bytes = build(opt_size, 0x1000, &valid_sections());
        assert_eq!(
            ParsedPe::parse(&bytes).unwrap_err(),
            PeError::TooSmall,
            "{opt_size}"
        );
    }
}

#[test]
fn e_lfanew_past_end_of_file_is_rejected() {
    let mut bytes = build(112, 0x1000, &valid_sections());
    bytes[0x3C..0x40].copy_from_slice(&u32::MAX.to_le_bytes());
    assert_eq!(ParsedPe::parse(&bytes).unwrap_err(), PeError::TooSmall);
}

#[test]
fn section_table_past_end_of_file_is_rejected() {
    let mut bytes = build(112, 0x1000, &valid_sections());
    bytes[0x46..0x48].copy_from_slice(&u16::MAX.to_le_bytes());
    assert_eq!(
        ParsedPe::parse(&bytes).unwrap_err(),
        PeError::InvalidSectionHeaders
    );
}

#[test]
fn section_range_overflowing_rva_space_is_rejected() {
    let mut secs = valid_sections();
    secs[1].va = 0xFFFF_F000;
    secs[1].vsize = 0x2000;
    let bytes = build(112, 0x1000, &secs);
    assert_eq!(
        ParsedPe::parse(&bytes).unwrap_err(),
        PeError::InvalidSectionHeaders
    );
}

/// Reproducer from #79: lookup over a section whose end overflows u32 must not panic or wrap.
#[test]
fn rva_lookup_with_overflowing_section_returns_none() {
    let bytes = build(112, 0x1000, &valid_sections());
    let mut pe = ParsedPe::parse(&bytes).unwrap();
    pe.sections.insert(
        0,
        PeSection {
            name: "wrap".into(),
            virtual_address: 0xFFFF_FF00,
            virtual_size: 0x200,
            raw_data_ptr: 0x200,
            raw_data_size: 0x200,
            characteristics: R,
            permissions: pe.sections[1].permissions,
        },
    );
    // A wrapped end (0xFFFF_FF00 + 0x200 = 0x100) would wrongly capture RVA 0x80.
    assert_eq!(pe.rva_to_file_offset(0x80), None);
    assert_eq!(pe.rva_to_file_offset(0xFFFF_FF10), Some(0x210));
    assert_eq!(pe.rva_to_file_offset(0x1004), Some(0x204));
}

#[test]
fn overlapping_sections_are_rejected() {
    let mut secs = valid_sections();
    secs[1].va = 0x1100; // .text maps 0x1000..0x1200 (raw size wins)
    let bytes = build(112, 0x1000, &secs);
    assert_eq!(
        ParsedPe::parse(&bytes).unwrap_err(),
        PeError::InvalidSectionHeaders
    );
}

#[test]
fn adjacent_sections_are_accepted() {
    let mut secs = valid_sections();
    secs[1].va = 0x1200;
    let bytes = build(112, 0x1000, &secs);
    assert!(ParsedPe::parse(&bytes).is_ok());
}

#[test]
fn truncated_section_data_is_rejected() {
    let mut secs = valid_sections();
    secs[1].raw_size = 0x201; // file is 0x600 bytes; 0x400 + 0x201 runs one byte past the end
    let bytes = build(112, 0x1000, &secs);
    assert_eq!(
        ParsedPe::parse(&bytes).unwrap_err(),
        PeError::InvalidSectionHeaders
    );

    secs[1].raw_size = 0x200;
    secs[1].raw_ptr = u32::MAX;
    let bytes = build(112, 0x1000, &secs);
    assert_eq!(
        ParsedPe::parse(&bytes).unwrap_err(),
        PeError::InvalidSectionHeaders
    );
}

#[test]
fn entry_point_outside_executable_section_is_rejected() {
    // Past the end of .text's mapped range, in non-executable .rdata, and outside every section.
    for entry in [0x1200, 0x2000, 0x0, u32::MAX] {
        let bytes = build(112, entry, &valid_sections());
        assert_eq!(
            ParsedPe::parse(&bytes).unwrap_err(),
            PeError::InvalidEntryPoint,
            "entry {entry:#x}"
        );
    }
    // Last byte of .text is still inside it.
    assert!(ParsedPe::parse(&build(112, 0x11FF, &valid_sections())).is_ok());
}
