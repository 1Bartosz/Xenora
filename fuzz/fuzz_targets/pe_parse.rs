// SPDX-License-Identifier: GPL-3.0-or-later
#![no_main]

use libfuzzer_sys::fuzz_target;

const MAX_MAPPED_BYTES: u64 = 64 * 1024 * 1024;

fuzz_target!(|data: &[u8]| {
    let Ok(pe) = xenora_loader::ParsedPe::parse(data) else {
        return;
    };

    let _ = pe.rva_to_file_offset(pe.entry_point_rva);
    let mut mapped_bytes = 0u64;
    for section in &pe.sections {
        let _ = pe.rva_to_file_offset(section.virtual_address);
        // The loader allocates each section separately, independent of size_of_image.
        let section_size = u64::from(section.virtual_size.max(section.raw_data_size));
        mapped_bytes = mapped_bytes.saturating_add(section_size);
    }

    if mapped_bytes <= MAX_MAPPED_BYTES && u64::from(pe.size_of_image) <= MAX_MAPPED_BYTES {
        let _ = pe.load_into_memory(data);
    }
});
