// SPDX-License-Identifier: GPL-3.0-or-later

mod demo;

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
        Some("demo") => match demo::run() {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("demo: {error}");
                ExitCode::FAILURE
            }
        },
        _ => {
            eprintln!("usage: xenora <info|demo>");
            ExitCode::from(2)
        }
    }
}
