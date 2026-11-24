// SPDX-License-Identifier: GPL-3.0-or-later

//! API-Set Redirection and Alias Resolution.
//!
//! Windows uses virtual DLL names (e.g. `api-ms-win-core-processthreads-l1-1-0.dll`)
//! that are redirected to canonical underlying modules (e.g. `kernel32.dll`, `KernelBase.dll`).
//!
//! This module resolves these virtual API-sets to canonical modules and vice-versa,
//! preventing duplicate stub generation and allowing unified handler lookup.
//! Source: Public Windows SDK `apisetcounters.h` and documented Microsoft API-set schema.

/// Resolve an API-set virtual DLL name to its canonical Windows module name.
pub fn resolve_api_set_canonical(module: &str) -> &'static str {
    let lower = module.to_ascii_lowercase();
    let trimmed = lower.strip_suffix(".dll").unwrap_or(&lower);

    if trimmed.starts_with("api-ms-win-core-processthreads")
        || trimmed.starts_with("api-ms-win-core-processtopology")
        || trimmed.starts_with("api-ms-win-core-profile")
        || trimmed.starts_with("api-ms-win-core-sysinfo")
        || trimmed.starts_with("api-ms-win-core-interlocked")
        || trimmed.starts_with("api-ms-win-core-libraryloader")
        || trimmed.starts_with("api-ms-win-core-namedpipe")
        || trimmed.starts_with("api-ms-win-core-handle")
        || trimmed.starts_with("api-ms-win-core-errorhandling")
        || trimmed.starts_with("api-ms-win-core-debug")
        || trimmed.starts_with("api-ms-win-core-datetime")
        || trimmed.starts_with("api-ms-win-core-timezone")
        || trimmed.starts_with("api-ms-win-core-processenvironment")
        || trimmed.starts_with("api-ms-win-core-file")
        || trimmed.starts_with("api-ms-win-core-io")
        || trimmed.starts_with("api-ms-win-core-synch")
        || trimmed.starts_with("api-ms-win-core-memory")
        || trimmed.starts_with("api-ms-win-core-heap")
        || trimmed.starts_with("api-ms-win-core-string")
        || trimmed.starts_with("api-ms-win-core-localization")
    {
        "kernel32.dll"
    } else if trimmed.starts_with("api-ms-win-core-rtlsupport") {
        "ntdll.dll"
    } else if trimmed.starts_with("api-ms-win-crt-") {
        "ucrtbase.dll"
    } else if trimmed == "vcruntime140" || trimmed == "vcruntime140_1" {
        "vcruntime140.dll"
    } else if trimmed == "msvcp140" || trimmed == "msvcp140_1" || trimmed == "msvcp140_2" {
        "msvcp140.dll"
    } else {
        match module.to_ascii_lowercase().as_str() {
            "kernel32.dll" | "kernelbase.dll" => "kernel32.dll",
            "ntdll.dll" => "ntdll.dll",
            "ws2_32.dll" => "ws2_32.dll",
            "advapi32.dll" => "advapi32.dll",
            _ => "unknown.dll",
        }
    }
}

/// Known API sets that forward to the given canonical host module.
pub fn get_api_sets_for_canonical(canonical: &str) -> &'static [&'static str] {
    match canonical {
        "kernel32.dll" => &[
            "api-ms-win-core-processthreads-l1-1-0.dll",
            "api-ms-win-core-processthreads-l1-1-1.dll",
            "api-ms-win-core-processthreads-l1-1-2.dll",
            "api-ms-win-core-synch-l1-1-0.dll",
            "api-ms-win-core-synch-l1-2-0.dll",
            "api-ms-win-core-synch-l1-2-1.dll",
            "api-ms-win-core-sysinfo-l1-1-0.dll",
            "api-ms-win-core-sysinfo-l1-2-0.dll",
            "api-ms-win-core-profile-l1-1-0.dll",
            "api-ms-win-core-libraryloader-l1-2-0.dll",
            "api-ms-win-core-libraryloader-l1-2-1.dll",
            "api-ms-win-core-file-l1-1-0.dll",
            "api-ms-win-core-file-l2-1-0.dll",
            "api-ms-win-core-errorhandling-l1-1-0.dll",
            "api-ms-win-core-debug-l1-1-0.dll",
            "api-ms-win-core-handle-l1-1-0.dll",
            "api-ms-win-core-interlocked-l1-1-0.dll",
            "api-ms-win-core-datetime-l1-1-1.dll",
            "api-ms-win-core-timezone-l1-1-0.dll",
        ],
        "ucrtbase.dll" => &[
            "api-ms-win-crt-math-l1-1-0.dll",
            "api-ms-win-crt-string-l1-1-0.dll",
            "api-ms-win-crt-stdio-l1-1-0.dll",
            "api-ms-win-crt-runtime-l1-1-0.dll",
            "api-ms-win-crt-heap-l1-1-0.dll",
            "api-ms-win-crt-time-l1-1-0.dll",
            "api-ms-win-crt-convert-l1-1-0.dll",
            "api-ms-win-crt-utility-l1-1-0.dll",
            "api-ms-win-crt-filesystem-l1-1-0.dll",
            "api-ms-win-crt-locale-l1-1-0.dll",
            "api-ms-win-crt-environment-l1-1-0.dll",
        ],
        _ => &[],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_api_set_resolution() {
        assert_eq!(
            resolve_api_set_canonical("api-ms-win-core-processthreads-l1-1-0.dll"),
            "kernel32.dll"
        );
        assert_eq!(
            resolve_api_set_canonical("api-ms-win-crt-math-l1-1-0.dll"),
            "ucrtbase.dll"
        );
        assert_eq!(
            resolve_api_set_canonical("api-ms-win-core-synch-l1-2-1.dll"),
            "kernel32.dll"
        );
    }
}
