// SPDX-License-Identifier: GPL-3.0-or-later

use std::fs;
use std::path::Path;
use std::process::{Command, Output};

fn inspect(path: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_xenora"))
        .arg("inspect")
        .arg(path)
        .output()
        .expect("launch Xenora")
}

#[test]
fn distinguishes_read_errors_from_invalid_pe_bytes() {
    let directory = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("inspect-diagnostics-{}", std::process::id()));
    fs::create_dir(&directory).expect("create test input directory");
    let missing = directory.join("missing.exe");
    let empty = directory.join("empty.exe");
    fs::write(&empty, []).expect("write empty input");

    let cases = [
        (inspect(&missing), "could not read executable: No such file"),
        (
            inspect(&directory),
            "could not read executable: Is a directory",
        ),
        (
            inspect(&empty),
            "executable image is smaller than required headers",
        ),
    ];
    fs::remove_dir_all(&directory).expect("remove test inputs");

    for (result, diagnostic) in cases {
        assert_eq!(result.status.code(), Some(1));
        assert!(result.stdout.is_empty());
        let stderr = String::from_utf8(result.stderr).expect("UTF-8 diagnostic");
        assert!(
            stderr.starts_with(&format!("inspect failed: {diagnostic}")),
            "{stderr}"
        );
        assert!(!stderr.contains(&*directory.to_string_lossy()), "{stderr}");
    }
}

#[test]
fn inspects_the_original_delay_import_fixture() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/pe/delay-imports.exe");
    let result = inspect(&fixture);
    assert_eq!(result.status.code(), Some(0));
    assert!(result.stderr.is_empty());
    let report = String::from_utf8(result.stdout).expect("UTF-8 report");
    assert!(
        report.starts_with("PE32+ Binary Triage Report:\n"),
        "{report}"
    );
    assert!(report.contains("Entry Point RVA:   0x00001000"), "{report}");
}
