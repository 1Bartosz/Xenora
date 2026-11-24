// SPDX-License-Identifier: GPL-3.0-or-later

use xenora_import_gen::{Classification, Manifest, SymbolDb, generate, parse_csv};

#[test]
fn test_builtin_db_classifies_crt_math() {
    let db = SymbolDb::builtin();
    let acos_cls = db.get("api-ms-win-crt-math-l1-1-0.dll", "acos");
    assert!(matches!(acos_cls, Some(Classification::CrtNative { .. })));

    let sqrtf_cls = db.get("api-ms-win-crt-math-l1-1-0.dll", "sqrtf");
    assert!(matches!(sqrtf_cls, Some(Classification::CrtNative { .. })));
}

#[test]
fn test_builtin_db_classifies_crt_string() {
    let db = SymbolDb::builtin();
    let strlen_cls = db.get("api-ms-win-crt-string-l1-1-0.dll", "strlen");
    assert!(matches!(strlen_cls, Some(Classification::CrtNative { .. })));

    let strcat_s_cls = db.get("api-ms-win-crt-string-l1-1-0.dll", "strcat_s");
    assert!(matches!(
        strcat_s_cls,
        Some(Classification::CrtWrapper { .. })
    ));
}

#[test]
fn test_builtin_db_case_insensitive() {
    let db = SymbolDb::builtin();
    let sym1 = db.get("API-MS-WIN-CRT-MATH-L1-1-0.DLL", "acos");
    let sym2 = db.get("api-ms-win-crt-math-l1-1-0.dll", "acos");
    assert_eq!(sym1, sym2);
}

#[test]
fn test_manifest_and_codegen_pipeline() {
    let csv_content = r#""module","symbol_or_ordinal","iat_rva"
"api-ms-win-crt-math-l1-1-0.dll","acos",0x1000
"api-ms-win-crt-string-l1-1-0.dll","strlen",0x1008
"api-ms-win-core-processthreads-l1-1-0.dll","CreateThread",0x1010
"custom.dll","UnknownCustomFunc",0x1018
"kernel32.dll","QueryPerformanceCounter",0x1020
"api-ms-win-core-profile-l1-1-0.dll","QueryPerformanceFrequency",0x1028
"api-ms-win-core-processthreads-l1-1-0.dll","GetThreadId",0x1030
"api-ms-win-core-handle-l1-1-0.dll","DuplicateHandle",0x1038
"api-ms-win-core-sysinfo-l1-1-0.dll","GetTickCount64",0x1040
"#;

    let tmp_dir = std::env::temp_dir().join("xenora_test_pipeline");
    std::fs::create_dir_all(&tmp_dir).unwrap();
    let csv_file = tmp_dir.join("test_imports.csv");
    std::fs::write(&csv_file, csv_content).unwrap();

    let records = parse_csv(csv_file.to_str().unwrap()).unwrap();
    assert_eq!(records.len(), 9);

    let db = SymbolDb::builtin();
    let manifest = Manifest::build(&records, &db);
    let summary = manifest.summary();

    assert_eq!(*summary.get("crt-native").unwrap_or(&0), 2);
    assert_eq!(*summary.get("win32-stub").unwrap_or(&0), 1);
    assert_eq!(*summary.get("manual").unwrap_or(&0), 1);

    assert_eq!(summary.get("standard-service"), Some(&5));
    assert_eq!(manifest.generatable().count(), 3);

    let files = generate(&manifest, &db);
    assert!(files.crt_rs.contains("register_generated_crt_services"));
    assert!(files.crt_rs.contains("strlen"));
    assert!(files.stubs_rs.contains("CreateThread"));
    assert!(!files.stubs_rs.contains("QueryPerformance"));
    assert!(!files.stubs_rs.contains("GetThreadId"));
    assert!(!files.stubs_rs.contains("DuplicateHandle"));
    assert!(!files.stubs_rs.contains("GetTickCount64"));
    assert!(files.tests_rs.contains("generated_test_strlen"));

    let out_dir = tmp_dir.join("out");
    files.write_all(out_dir.to_str().unwrap()).unwrap();
    assert!(out_dir.join("generated_crt.rs").exists());
    assert!(out_dir.join("generated_stubs.rs").exists());
    assert!(out_dir.join("generated_tests.rs").exists());
    assert!(out_dir.join("generated_manifest.md").exists());
}
