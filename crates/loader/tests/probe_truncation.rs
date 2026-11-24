use xenora_loader::probe::{ProbeConfig, probe_pe_bytes};

/// The static probe must never panic on a truncated PE.
///
/// The fixture is truncated across its whole length, which covers the range
/// where the import directory and the import names overlap the end of the
/// file, and every other range where a name or thunk read computed from the
/// section header can run past the buffer.
#[test]
fn probe_never_panics_on_truncated_input() {
    let pe = include_bytes!("../../../fixtures/pe/trusted-runner.exe");
    for len in 1..=pe.len() {
        let truncated = &pe[..len];
        // The probe may reject the file or report it with unreadable
        // imports, but it must not panic.
        let _ = probe_pe_bytes(truncated, &ProbeConfig::default());
    }
}
