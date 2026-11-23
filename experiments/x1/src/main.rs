// SPDX-License-Identifier: GPL-3.0-or-later
#![deny(unsafe_code)]

mod cpu;
mod fixture;
#[cfg(feature = "metal")]
#[allow(unsafe_code)]
mod graphics;
mod interp;
mod loader;
mod mem;
mod pe;
#[cfg(any(test, feature = "metal"))]
mod x3;
#[cfg(any(test, feature = "metal"))]
mod x4;
mod x5;
mod x6;
mod x7;
#[cfg(feature = "metal")]
mod x8;
#[cfg(feature = "metal")]
mod x9;

fn main() -> std::process::ExitCode {
    let mut args = std::env::args_os().skip(1);
    let experiment = args.next();
    match experiment.as_deref().and_then(std::ffi::OsStr::to_str) {
        Some("x7") => return run_pe(&mut args, x7::Profile::Imports, "x7"),
        Some("x9") => {
            #[cfg(feature = "metal")]
            return run_pe(&mut args, x7::Profile::Resources, "x9");
            #[cfg(not(feature = "metal"))]
            {
                eprintln!("X9 requires the metal feature");
                return std::process::ExitCode::FAILURE;
            }
        }
        _ => {}
    }
    if args.next().is_some() {
        eprintln!("expected at most one experiment argument");
        return std::process::ExitCode::FAILURE;
    }
    match experiment.as_deref().and_then(std::ffi::OsStr::to_str) {
        Some("x2") => return run_x2(),
        Some("x6") => return x6::run(),
        Some("x5") => return x5::run(),
        Some("x4") => {
            #[cfg(feature = "metal")]
            return x4::run();
            #[cfg(not(feature = "metal"))]
            {
                eprintln!("X4 requires the metal feature");
                return std::process::ExitCode::FAILURE;
            }
        }
        Some("x3") => {
            #[cfg(feature = "metal")]
            return x3::run();
            #[cfg(not(feature = "metal"))]
            {
                eprintln!("X3 requires the metal feature");
                return std::process::ExitCode::FAILURE;
            }
        }
        _ if experiment.is_none() => {}
        _ => {
            eprintln!("unknown experiment {:?}", experiment.unwrap_or_default());
            return std::process::ExitCode::FAILURE;
        }
    }
    for (name, bytes) in [("wrap", fixture::WRAP), ("nonwrap", fixture::NONWRAP)] {
        let mut state = fixture::initial();
        let mut output = fixture::OUTPUT;
        println!("{name} bytes={bytes:02x?} initial={state:x?} output={output:02x?}");
        let result = cpu::run(
            bytes,
            &mut state,
            fixture::OUTPUT_BASE,
            &mut output,
            16,
            |state| {
                println!("retired {state:x?}");
            },
        );
        println!("terminal={state:x?} output={output:02x?} result={result:x?}");
        if let Err(fault) = result {
            eprintln!("fault {:?} at guest RIP {:#x}", fault.kind, fault.rip);
            return std::process::ExitCode::FAILURE;
        }
    }
    std::process::ExitCode::SUCCESS
}

fn run_x2() -> std::process::ExitCode {
    let mut image = match pe::load(pe::FIXTURE) {
        Ok(image) => image,
        Err(error) => {
            eprintln!("X2 load failed: {error:?}");
            return std::process::ExitCode::FAILURE;
        }
    };
    let mut state = image.initial(41);
    println!("X2 initial={state:x?} data={:02x?}", image.data);
    let result = image.run(&mut state, 16, |state| println!("retired {state:x?}"));
    println!(
        "terminal={state:x?} data={:02x?} result={result:x?}",
        image.data
    );
    if result.is_ok() {
        std::process::ExitCode::SUCCESS
    } else {
        std::process::ExitCode::FAILURE
    }
}

fn run_pe(
    args: &mut impl Iterator<Item = std::ffi::OsString>,
    profile: x7::Profile,
    command: &str,
) -> std::process::ExitCode {
    match (args.next(), args.next()) {
        (Some(path), None) => x7::run(std::path::Path::new(&path), profile),
        _ => {
            eprintln!("usage: xenora-x1 {command} <synthetic-pe-path>");
            std::process::ExitCode::FAILURE
        }
    }
}
