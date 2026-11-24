// SPDX-License-Identifier: GPL-3.0-or-later

//! Generate the typed runtime adapters and report. Run from the repository root.
//! `--check` regenerates in memory and fails if a committed file differs.

use std::process::ExitCode;
use xenora_import_gen::typed::{generate_report, generate_rust, parse};

const REGISTRY: &str = "crates/loader/runtime/services.txt";
const TESTS: &str = "crates/loader/tests/runtime_services.rs";
const RUST: &str = "crates/loader/src/runtime_table.rs";
const REPORT: &str = "docs/runtime/services.md";

fn run() -> Result<String, String> {
    let mode = std::env::args().nth(1).unwrap_or_default();
    let read = |path: &str| std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"));
    let services =
        parse(&read(REGISTRY)?, &read(TESTS)?).map_err(|e| format!("{REGISTRY}: {e}"))?;
    let outputs = [
        (RUST, generate_rust(&services)),
        (REPORT, generate_report(&services)),
    ];
    for (path, text) in &outputs {
        match mode.as_str() {
            "generate" => std::fs::write(path, text).map_err(|e| format!("{path}: {e}"))?,
            "--check" if read(path)? != *text => {
                return Err(format!(
                    "{path} is out of date; run xenora-runtime-gen generate"
                ));
            }
            "--check" | "validate" => {}
            _ => return Err("usage: xenora-runtime-gen validate|generate|--check".into()),
        }
    }
    Ok(format!("{} services", services.len()))
}

fn main() -> ExitCode {
    match run() {
        Ok(message) => {
            println!("{message}");
            ExitCode::SUCCESS
        }
        Err(message) => {
            eprintln!("{message}");
            ExitCode::FAILURE
        }
    }
}
