//! LLVM oracle for instruction identification: names and lengths from llvm-mc encodings.

use std::process::Command;

use xenora_isa::meta::identify;

const SOURCE: &str = include_str!("llvm/identify.s");

fn encodings() -> Vec<(String, Vec<u8>)> {
    let executable = std::env::var_os("LLVM_BIN").map_or_else(
        || "llvm-mc".into(),
        |directory| std::path::PathBuf::from(directory).join("llvm-mc"),
    );
    let path = std::env::temp_dir().join(format!("xenora-identify-{}.s", std::process::id()));
    std::fs::write(&path, SOURCE).unwrap();
    let output = Command::new(executable)
        .args([
            "-triple=amdgcn-amd-amdhsa",
            "-mcpu=gfx1030",
            "-mattr=+wavefrontsize32",
            "-show-encoding",
        ])
        .arg(&path)
        .output()
        .expect("launch llvm-mc (provide its directory in LLVM_BIN or put it on PATH)");
    std::fs::remove_file(&path).ok();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .filter_map(|line| {
            let (text, encoding) = line.split_once("; encoding: [")?;
            let mnemonic = text.split_whitespace().next()?.to_ascii_uppercase();
            let bytes = encoding
                .trim_end_matches(']')
                .split(',')
                .map(|byte| u8::from_str_radix(byte.trim().trim_start_matches("0x"), 16).unwrap())
                .collect();
            Some((mnemonic, bytes))
        })
        .collect()
}

fn base_name(mnemonic: &str) -> &str {
    ["_E32", "_E64", "_SDWA", "_DPP"]
        .iter()
        .fold(mnemonic, |name, suffix| name.trim_end_matches(suffix))
}

#[test]
#[ignore = "requires LLVM with the gfx1030 AMDGPU target"]
fn identification_matches_llvm_names_and_lengths() {
    let encodings = encodings();
    let expected_lines = SOURCE
        .lines()
        .filter(|line| !line.is_empty() && !line.starts_with("//"))
        .count();
    assert_eq!(encodings.len(), expected_lines);
    let mut stream = Vec::new();
    for (mnemonic, bytes) in &encodings {
        let found = identify(bytes).unwrap_or_else(|error| panic!("{mnemonic}: {error}"));
        assert_eq!(found.form.name, base_name(mnemonic), "{mnemonic}");
        assert_eq!(found.len, bytes.len(), "{mnemonic}");
        stream.extend_from_slice(bytes);
    }
    let mut offset = 0;
    for (mnemonic, bytes) in &encodings {
        let found = identify(&stream[offset..]).unwrap();
        assert_eq!(found.len, bytes.len(), "{mnemonic} in stream");
        offset += found.len;
    }
    assert_eq!(offset, stream.len());
}
