// SPDX-License-Identifier: GPL-3.0-or-later

//! Automated C / POSIX header parser for metadata ingestion.
//!
//! Ingests standard system headers (e.g. `/usr/include/math.h`, `_string.h`)
//! and extracts function signatures into typed `FunctionSignature` definitions.
//!
//! Preserves clean-room provenance and licensing attribution.

use crate::metadata::{AbiType, CallingConvention, FunctionSignature, MetadataCatalog, ParamDesc};
use std::path::Path;

/// Ingest C header files from disk into `catalog`.
pub fn ingest_header_file(
    catalog: &mut MetadataCatalog,
    module: &str,
    header_path: &Path,
) -> Result<usize, String> {
    let content = std::fs::read_to_string(header_path)
        .map_err(|e| format!("cannot read {}: {e}", header_path.display()))?;

    let filename = header_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("header.h");

    let count = parse_c_declarations(catalog, module, filename, &content);
    Ok(count)
}

/// Ingest standard macOS SDK math and string headers if available.
pub fn ingest_standard_system_headers(catalog: &mut MetadataCatalog) -> usize {
    let candidates = [
        (
            "ucrtbase.dll",
            "/Library/Developer/CommandLineTools/SDKs/MacOSX.sdk/usr/include/math.h",
        ),
        (
            "ucrtbase.dll",
            "/Library/Developer/CommandLineTools/SDKs/MacOSX.sdk/usr/include/_string.h",
        ),
    ];

    let mut total = 0;
    for (mod_name, path_str) in candidates {
        let p = Path::new(path_str);
        if p.exists()
            && let Ok(n) = ingest_header_file(catalog, mod_name, p)
        {
            total += n;
        }
    }
    total
}

/// Parse C function prototypes from `content`.
fn parse_c_declarations(
    catalog: &mut MetadataCatalog,
    module: &str,
    header_name: &str,
    content: &str,
) -> usize {
    let mut added = 0;

    for line in content.lines() {
        let line = line.trim();
        // Look for lines containing function prototypes, e.g. "extern double acos(double);" or "int strcmp(...);"
        if !line.contains('(')
            || !line.contains(')')
            || line.starts_with('#')
            || line.starts_with("/*")
            || line.starts_with('*')
        {
            continue;
        }

        // Clean up __attribute__, extern, etc.
        let cleaned = line
            .replace("extern ", "")
            .replace("__DARWIN_ALIAS(", "/* alias */ ")
            .replace("__DARWIN_ALIAS_C(", "/* alias */ ")
            .replace("_LIBC_CSTR", "char *")
            .replace("_LIBC_UNSAFE_INDEXABLE", "");

        let Some(open_paren) = cleaned.find('(') else {
            continue;
        };
        let Some(close_paren) = cleaned[open_paren..].find(')') else {
            continue;
        };
        let close_paren = open_paren + close_paren;

        let decl_left = cleaned[..open_paren].trim();
        let args_str = cleaned[open_paren + 1..close_paren].trim();

        // Split return type and function name
        let parts: Vec<&str> = decl_left.split_whitespace().collect();
        if parts.len() < 2 {
            continue;
        }
        let ret_str = parts[..parts.len() - 1].join(" ");
        let name_str = parts[parts.len() - 1].trim_start_matches('*');

        if !is_valid_c_ident(name_str) {
            continue;
        }

        let ret_type = match parse_c_type(&ret_str) {
            Some(t) => t,
            None => continue,
        };

        let mut params = Vec::new();
        if !args_str.is_empty() && args_str != "void" {
            for (idx, arg) in args_str.split(',').enumerate() {
                let arg_clean = arg.trim();
                let arg_parts: Vec<&str> = arg_clean.split_whitespace().collect();
                let (param_name, type_str) = if arg_parts.is_empty() {
                    (format!("arg{idx}"), "void".to_string())
                } else if arg_parts.len() == 1 {
                    (format!("arg{idx}"), arg_parts[0].to_string())
                } else {
                    let last = arg_parts[arg_parts.len() - 1].trim_start_matches('*');
                    if is_valid_c_ident(last) {
                        (last.to_string(), arg_parts[..arg_parts.len() - 1].join(" "))
                    } else {
                        (format!("arg{idx}"), arg_clean.to_string())
                    }
                };

                let param_type = parse_c_type(&type_str).unwrap_or(AbiType::U64);
                params.push(ParamDesc {
                    name: param_name,
                    abi_type: param_type,
                });
            }
        }

        let sig = FunctionSignature {
            name: name_str.to_string(),
            return_type: ret_type,
            params,
            convention: CallingConvention::Win64,
            header_source: format!("<{header_name}>"),
            doc: format!("Ingested from {header_name}"),
        };

        catalog
            .entries
            .insert((module.to_ascii_lowercase(), name_str.to_string()), sig);
        added += 1;
    }

    added
}

fn is_valid_c_ident(s: &str) -> bool {
    if s.is_empty() {
        return false;
    }
    let bytes = s.as_bytes();
    if !bytes[0].is_ascii_alphabetic() && bytes[0] != b'_' {
        return false;
    }
    bytes
        .iter()
        .all(|&b| b.is_ascii_alphanumeric() || b == b'_')
}

fn parse_c_type(s: &str) -> Option<AbiType> {
    let s = s.trim();
    if s == "void" {
        Some(AbiType::Void)
    } else if s == "int" || s == "int32_t" {
        Some(AbiType::I32)
    } else if s == "unsigned int" || s == "uint32_t" || s == "unsigned" {
        Some(AbiType::U32)
    } else if s == "long long" || s == "int64_t" {
        Some(AbiType::I64)
    } else if s == "size_t" || s == "unsigned long" || s == "unsigned long long" || s == "uint64_t"
    {
        Some(AbiType::U64)
    } else if s == "float" {
        Some(AbiType::F32)
    } else if s == "double" {
        Some(AbiType::F64)
    } else if s.contains("const char") || s.contains("char *") {
        Some(AbiType::StringAnsi)
    } else if s.contains('*') {
        Some(AbiType::Pointer {
            is_const: s.contains("const"),
            pointee: "void".into(),
        })
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_declarations_from_string() {
        let mut cat = MetadataCatalog::new();
        let c_code = r#"
            extern double cos(double x);
            extern double atan2(double y, double x);
            int strcmp(const char *s1, const char *s2);
        "#;
        let count = parse_c_declarations(&mut cat, "ucrtbase.dll", "test.h", c_code);
        assert_eq!(count, 3);
        assert!(cat.lookup("ucrtbase.dll", "cos").is_some());
        assert!(cat.lookup("ucrtbase.dll", "atan2").is_some());
        assert!(cat.lookup("ucrtbase.dll", "strcmp").is_some());
    }
}
