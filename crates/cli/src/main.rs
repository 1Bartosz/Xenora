// SPDX-License-Identifier: GPL-3.0-or-later

mod demo;
mod runner;

use std::process::ExitCode;

fn main() -> ExitCode {
    match std::env::args().nth(1).as_deref() {
        Some("info") => {
            println!(
                "Xenora {} ({})",
                env!("CARGO_PKG_VERSION"),
                std::env::consts::ARCH
            );
            ExitCode::SUCCESS
        }
        Some("inspect") => {
            let Some(file_path) = std::env::args().nth(2) else {
                eprintln!("usage: xenora inspect <path-to-pe-binary>");
                return ExitCode::from(2);
            };
            match xenora_loader::triage_file(&file_path) {
                Ok(report) => {
                    print!("{report}");
                    ExitCode::SUCCESS
                }
                Err(err) => {
                    eprintln!("inspect failed: {err}");
                    ExitCode::FAILURE
                }
            }
        }
        Some("demo") => match demo::run() {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("demo: {error}");
                ExitCode::FAILURE
            }
        },
        Some("run") => runner::command(std::env::args_os().skip(2)),
        Some("imports") => imports_command(std::env::args().skip(2).collect()),
        Some("probe") => {
            let args: Vec<String> = std::env::args().collect();
            let mut pe_path: Option<String> = None;
            let mut mode = "static".to_string();
            let mut report_dir = "diagnostics/gta6-candidate-e95020b2".to_string();

            let mut i = 2;
            while i < args.len() {
                match args[i].as_str() {
                    "--pe" => {
                        if i + 1 < args.len() {
                            pe_path = Some(args[i + 1].clone());
                            i += 2;
                        } else {
                            eprintln!("missing argument for --pe");
                            return ExitCode::from(2);
                        }
                    }
                    "--mode" => {
                        if i + 1 < args.len() {
                            mode = args[i + 1].clone();
                            i += 2;
                        } else {
                            eprintln!("missing argument for --mode");
                            return ExitCode::from(2);
                        }
                    }
                    "--report" => {
                        if i + 1 < args.len() {
                            report_dir = args[i + 1].clone();
                            i += 2;
                        } else {
                            eprintln!("missing argument for --report");
                            return ExitCode::from(2);
                        }
                    }
                    other => {
                        eprintln!("unknown probe argument: {other}");
                        return ExitCode::from(2);
                    }
                }
            }

            let Some(pe) = pe_path else {
                eprintln!(
                    "usage: xenora probe --pe <path> [--mode <static>] [--report <output-dir>]"
                );
                return ExitCode::from(2);
            };

            if mode != "static" {
                eprintln!("error: currently only --mode static is supported");
                return ExitCode::from(2);
            }

            let bytes = match std::fs::read(&pe) {
                Ok(b) => b,
                Err(err) => {
                    eprintln!("error reading PE binary '{pe}': {err}");
                    return ExitCode::FAILURE;
                }
            };

            let config = xenora_loader::ProbeConfig::default();
            match xenora_loader::run_probe_and_export(&bytes, &report_dir, &config) {
                Ok(report) => {
                    println!("Xenora static PE probe complete.");
                    println!("Artifacts exported to: {report_dir}");
                    println!("Sections: {}", report.parsed.sections.len());
                    println!("Relocations: {}", report.relocations.total_entries);
                    println!("TLS Callbacks: {}", report.tls.callback_targets.len());
                    println!("Imported Thunks: {}", report.imports.len());
                    println!(
                        "Unresolved Standard Import Thunks: {}",
                        report
                            .imports
                            .iter()
                            .filter(|import| !import.is_supported)
                            .count()
                    );
                    if let Some(insn) = report.first_unsupported_dbt_insn {
                        println!("First DBT Blocker: RVA 0x{:08x} ({})", insn.rva, insn.notes);
                    }
                    ExitCode::SUCCESS
                }
                Err(err) => {
                    eprintln!("probe failed: {err}");
                    ExitCode::FAILURE
                }
            }
        }
        _ => {
            eprintln!("usage: xenora <info|demo|inspect|probe|run|imports>");
            ExitCode::from(2)
        }
    }
}

/// `xenora imports sync [--report <dir>] [--check]`
///
/// Reads the `imports.csv` from a probe report directory, compares each import
/// against the current standard service registry, and writes a refreshed
/// `import_backlog.md` and `import_coverage.csv` with implementation status.
///
/// With `--check`: exits non-zero if `import_backlog.md` would change.
fn imports_command(args: Vec<String>) -> ExitCode {
    let mut report_dir = "diagnostics/gta6-candidate-e95020b2".to_string();
    let mut check_mode = false;
    let mut subcommand_seen = false;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "sync" => {
                subcommand_seen = true;
                i += 1;
            }
            "--report" => {
                if i + 1 < args.len() {
                    report_dir = args[i + 1].clone();
                    i += 2;
                } else {
                    eprintln!("missing argument for --report");
                    return ExitCode::from(2);
                }
            }
            "--check" => {
                check_mode = true;
                i += 1;
            }
            other => {
                eprintln!("unknown imports argument: {other}");
                return ExitCode::from(2);
            }
        }
    }

    if !subcommand_seen {
        eprintln!("usage: xenora imports sync [--report <dir>] [--check]");
        return ExitCode::from(2);
    }

    match run_imports_sync(&report_dir, check_mode) {
        Ok(changed) => {
            if check_mode {
                if changed {
                    eprintln!("imports sync --check: import_backlog.md is out of date");
                    ExitCode::FAILURE
                } else {
                    println!("imports sync --check: import_backlog.md is up to date");
                    ExitCode::SUCCESS
                }
            } else {
                println!("imports sync complete. Report written to: {report_dir}");
                ExitCode::SUCCESS
            }
        }
        Err(e) => {
            eprintln!("imports sync failed: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run_imports_sync(
    report_dir: &str,
    check_mode: bool,
) -> Result<bool, Box<dyn std::error::Error>> {
    use std::collections::{BTreeMap, BTreeSet};
    use std::sync::Arc;
    use xenora_loader::ImportRegistry;
    use xenora_loader::imports::ServiceStatus;
    use xenora_loader::services::{MockClock, ServicesState, register_standard_services};

    let imports_csv_path = std::path::Path::new(report_dir).join("imports.csv");
    if !imports_csv_path.exists() {
        return Err(
            format!("imports.csv not found in '{report_dir}'. Run `xenora probe` first.").into(),
        );
    }

    // Build the standard registry to get current implementation state.
    let mut registry = ImportRegistry::new();
    register_standard_services(
        &mut registry,
        Arc::new(ServicesState::new(Arc::new(MockClock::new(0)))),
    );

    // Parse imports.csv (skip header line).
    let csv_text = std::fs::read_to_string(&imports_csv_path)?;
    let mut by_module: BTreeMap<String, Vec<(String, bool, Option<ServiceStatus>)>> =
        BTreeMap::new();
    let mut total = 0usize;
    let mut counts: BTreeMap<&'static str, usize> = BTreeMap::new();

    for line in csv_text.lines().skip(1) {
        // Format: "module","symbol",0xRVA,category,registered,status,is_alias,"canonical",priority,cluster
        // We re-query the live registry rather than trusting cached status.
        let fields: Vec<&str> = line.splitn(10, ',').collect();
        if fields.len() < 2 {
            continue;
        }
        let module = fields[0].trim_matches('"').to_string();
        let symbol = fields[1].trim_matches('"').to_string();
        if module.is_empty() || symbol.is_empty() {
            continue;
        }

        let binding = registry.binding(&module, &symbol);
        let is_registered = binding.is_some();
        let status = binding.map(|b| b.status);

        let label: &'static str = match status {
            Some(ServiceStatus::Implemented) => "implemented",
            Some(ServiceStatus::Partial) => "partial",
            Some(ServiceStatus::Stub) => "stub",
            None => {
                // Check explicit known-unsupported list
                match (module.to_ascii_lowercase().as_str(), symbol.as_str()) {
                    (
                        "api-ms-win-core-processthreads-l1-1-0.dll",
                        "CreateThread" | "ResumeThread" | "SuspendThread" | "TerminateProcess",
                    )
                    | (
                        "api-ms-win-core-synch-l1-1-0.dll",
                        "WaitForSingleObject" | "WaitForSingleObjectEx",
                    )
                    | ("api-ms-win-core-synch-l1-2-1.dll", "WaitForMultipleObjects")
                    | ("api-ms-win-core-rtlsupport-l1-1-0.dll", "RtlVirtualUnwind") => {
                        "unsupported"
                    }
                    _ => "missing",
                }
            }
        };

        *counts.entry(label).or_default() += 1;
        total += 1;
        by_module
            .entry(module.to_ascii_lowercase())
            .or_default()
            .push((symbol, is_registered, status));
    }

    // Build the refreshed import_backlog.md.
    let mut md = String::new();
    let _ = writeln!(md, "# Xenora import compatibility backlog");
    let _ = writeln!(md);
    let _ = writeln!(
        md,
        "Generated by `xenora imports sync`. Do not edit manually."
    );
    let _ = writeln!(
        md,
        "Re-run after adding service handlers to refresh this file."
    );
    let _ = writeln!(md);
    let _ = writeln!(md, "## Summary");
    let _ = writeln!(md);
    let _ = writeln!(md, "| Status | Count |");
    let _ = writeln!(md, "| --- | ---: |");
    let _ = writeln!(md, "| **Total** | {total} |");
    for (label, count) in &counts {
        let _ = writeln!(md, "| {label} | {count} |");
    }
    let _ = writeln!(md);

    let _ = writeln!(md, "## Missing handlers (highest value first)");
    let _ = writeln!(md);

    // Priority ordering: processthreads, synch, rtlsupport, memory, crt, file, graphics, other
    fn priority_key(module: &str) -> u8 {
        if module.contains("processthreads") || module.contains("core-handle") {
            1
        } else if module.contains("fibers") || module.contains("tls") {
            2
        } else if module.contains("core-synch") {
            3
        } else if module.contains("core-profile") || module.contains("core-sysinfo") {
            4
        } else if module.contains("core-memory")
            || module.contains("core-heap")
            || module == "xmem.dll"
        {
            5
        } else if module.starts_with("api-ms-win-crt-")
            || module.contains("vcruntime")
            || module.contains("libraryloader")
        {
            6
        } else if module.contains("rtlsupport") || module.contains("errorhandling") {
            7
        } else if module.contains("core-file") || module == "xcrdapi.dll" {
            8
        } else {
            9
        }
    }

    // Collect missing/unsupported into a prioritized list.
    let mut missing_entries: Vec<(u8, String, String)> = Vec::new();
    for (module, symbols) in &by_module {
        for (symbol, is_registered, _status) in symbols {
            if !is_registered {
                missing_entries.push((priority_key(module), module.clone(), symbol.clone()));
            }
        }
    }
    missing_entries.sort();

    let mut prev_priority = 255u8;
    for (priority, module, symbol) in &missing_entries {
        if *priority != prev_priority {
            let _ = writeln!(md, "### Priority {priority}");
            let _ = writeln!(md);
            prev_priority = *priority;
        }
        let _ = writeln!(md, "- `{module}!{symbol}`");
    }

    let _ = writeln!(md);
    let _ = writeln!(md, "## Registered handlers");
    let _ = writeln!(md);
    let _ = writeln!(md, "| Module | Symbol | Status |");
    let _ = writeln!(md, "| --- | --- | --- |");
    // Enumerate canonical registrations sorted by module, symbol
    let mut registered: BTreeSet<(String, String, &'static str)> = BTreeSet::new();
    for (module, symbol, status) in registry.iter_registered() {
        registered.insert((
            module.to_ascii_lowercase(),
            symbol.to_string(),
            status.as_str(),
        ));
    }
    for (module, symbol, status) in &registered {
        let _ = writeln!(md, "| `{module}` | `{symbol}` | {status} |");
    }

    let out_path = std::path::Path::new(report_dir).join("import_backlog.md");
    let existing = std::fs::read_to_string(&out_path).unwrap_or_default();
    let changed = existing != md;

    if !check_mode {
        std::fs::write(&out_path, &md)?;
        println!(
            "  Total imports: {total} | implemented: {} | partial: {} | stub: {} | missing: {}",
            counts.get("implemented").copied().unwrap_or(0),
            counts.get("partial").copied().unwrap_or(0),
            counts.get("stub").copied().unwrap_or(0),
            counts.get("missing").copied().unwrap_or(0),
        );
    }

    Ok(changed)
}

use std::fmt::Write as _;
