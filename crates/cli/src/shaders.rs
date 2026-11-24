//! `xenora shaders`: instruction inventory and stream inspection.

use std::process::ExitCode;

use xenora_decoder::{Architecture, SEMANTIC_FORMS, decode};
use xenora_isa::meta::{self, EncodingFamily, FORMS, identify};

const USAGE: &str = "usage: xenora shaders <coverage [--json] | inspect <gfx1030.bin>>";

pub fn command(args: &[String]) -> ExitCode {
    match args.first().map(String::as_str) {
        Some("coverage") => coverage(args.get(1).map(String::as_str) == Some("--json")),
        Some("inspect") => match args.get(1) {
            Some(path) => inspect(path),
            None => usage(),
        },
        _ => usage(),
    }
}

fn usage() -> ExitCode {
    eprintln!("{USAGE}");
    ExitCode::from(2)
}

fn semantic(family: EncodingFamily) -> usize {
    SEMANTIC_FORMS
        .iter()
        .filter(|(_, form_family)| *form_family == family)
        .count()
}

/// Per-family counts. "Discovered" forms come from AMD's XML and are identified
/// with their length; "semantic" forms decode into executable instructions.
/// Reference, Metal and native GPU evidence per form stays in docs/status.toml.
fn coverage(json: bool) -> ExitCode {
    let rows: Vec<_> = meta::inventory()
        .into_iter()
        .map(|(family, discovered)| (family, discovered, semantic(family)))
        .collect();
    let total: usize = rows.iter().map(|row| row.1).sum();
    let supported: usize = rows.iter().map(|row| row.2).sum();
    if json {
        let families: Vec<String> = rows
            .iter()
            .map(|(family, discovered, semantic)| {
                format!(
                    "{{\"family\":\"{}\",\"discovered\":{discovered},\"semantic\":{semantic}}}",
                    family.name()
                )
            })
            .collect();
        println!(
            "{{\"architecture\":\"gfx1030\",\"source_sha256\":\"{}\",\"discovered\":{total},\"semantic\":{supported},\"families\":[{}]}}",
            meta::SOURCE_SHA256,
            families.join(",")
        );
    } else {
        println!("{:<10} {:>10} {:>9}", "family", "discovered", "semantic");
        for (family, discovered, semantic) in &rows {
            println!("{:<10} {discovered:>10} {semantic:>9}", family.name());
        }
        println!("{:<10} {total:>10} {supported:>9}", "total");
        println!("Semantic forms decode for the operand subsets in docs/status.toml only.");
    }
    ExitCode::SUCCESS
}

fn inspect(path: &str) -> ExitCode {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) => {
            eprintln!("{path}: {error}");
            return ExitCode::FAILURE;
        }
    };
    let mut offset = 0;
    let mut unsupported = 0;
    let mut count = 0;
    while offset < bytes.len() {
        let found = match identify(&bytes[offset..]) {
            Ok(found) => found,
            Err(error) => {
                println!("{offset:#06x}  decode failure: {error}");
                return ExitCode::FAILURE;
            }
        };
        let words: Vec<String> = bytes[offset..offset + found.len]
            .chunks(4)
            .map(|chunk| format!("{:08x}", u32::from_le_bytes(chunk.try_into().unwrap())))
            .collect();
        let status = match decode(Architecture::Gfx1030, &bytes[offset..offset + found.len]) {
            Ok(_) => "semantic".to_owned(),
            Err(error) if SEMANTIC_FORMS.contains(&(found.form.name, found.form.family)) => {
                format!("unsupported operands: {error}")
            }
            Err(_) => "unsupported instruction semantics".to_owned(),
        };
        if status != "semantic" {
            unsupported += 1;
        }
        println!(
            "{offset:#06x}  {:<8} {:<28} {:<26} {status}",
            found.form.family.name(),
            found.form.name.to_ascii_lowercase(),
            words.join(" ")
        );
        offset += found.len;
        count += 1;
    }
    println!(
        "{count} instructions inspected against {} gfx1030 forms; {unsupported} not executable",
        FORMS.len()
    );
    ExitCode::SUCCESS
}
