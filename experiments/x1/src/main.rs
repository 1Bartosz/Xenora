// SPDX-License-Identifier: GPL-3.0-or-later
#![deny(unsafe_code)]

mod cpu;
mod fixture;
#[cfg(feature = "metal")]
#[allow(unsafe_code)]
mod graphics;
mod pe;
#[cfg(any(test, feature = "metal"))]
mod x3;
#[cfg(any(test, feature = "metal"))]
mod x4;

fn main() -> std::process::ExitCode {
    match std::env::args().nth(1).as_deref() {
        Some("x2") => return run_x2(),
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
        _ => {}
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
