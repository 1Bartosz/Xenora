// SPDX-License-Identifier: GPL-3.0-or-later

//! Verification and conformance test runner for import bindings.
//!
//! Runs deterministic synthetic checks against all generated bindings
//! to ensure they adhere to Windows x64 ABI standards and guest memory safety.

use std::process::Command;

/// Result of running the verification suite.
#[derive(Debug)]
pub struct VerificationResult {
    pub passed: usize,
    pub failed: usize,
    pub total: usize,
    pub details: Vec<String>,
}

/// Run automated behavioral and conformance tests for generated bindings.
pub fn run_verification() -> Result<VerificationResult, String> {
    let output = Command::new("cargo")
        .args([
            "test",
            "-p",
            "xenora-loader",
            "--test",
            "generated_conformance",
        ])
        .output()
        .map_err(|e| format!("failed to execute cargo test: {e}"))?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    let mut passed = 0;
    let mut failed = 0;
    let mut details = Vec::new();

    for line in stdout.lines() {
        if line.starts_with("test generated_test_") || line.starts_with("test generated_stubs_") {
            details.push(line.to_string());
            if line.ends_with("... ok") {
                passed += 1;
            } else if line.ends_with("... FAILED") {
                failed += 1;
            }
        }
    }

    if !output.status.success() && failed == 0 {
        return Err(format!("cargo test execution failed:\n{stderr}\n{stdout}"));
    }

    Ok(VerificationResult {
        passed,
        failed,
        total: passed + failed,
        details,
    })
}
