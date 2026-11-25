// SPDX-License-Identifier: GPL-3.0-or-later

use std::error::Error;
use std::ffi::OsString;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use sha2::{Digest, Sha256};
use xenora_loader::startup::{StartupEvent, StartupLayout, run_synthetic_startup_with_observer};
use xenora_loader::{
    ExitReason, ImportRegistry, Register, ServicesState, SystemClock, register_standard_services,
};
use xenora_metal::NativeCompute;
use xenora_shader::{Architecture, PIXEL_ENTRY_POINT, VERTEX_ENTRY_POINT, compile_graphics};

const FIXTURE: &[u8] = include_bytes!("../../../fixtures/pe/trusted-runner.exe");
const SERVICES_FIXTURE: &[u8] = include_bytes!("../../../fixtures/pe/runtime-services.exe");
const WALL_LIMIT: Duration = Duration::from_secs(15);
const DEFAULT_STEPS: usize = 100;
const MAX_STEPS: usize = 10_000;
const IMAGE_BASE: u64 = 0x150000000;
const TICKET_ADDRESS: u64 = IMAGE_BASE + 0x3000;
const TICKET: u64 = 17;
const DRAW_RESULT: u64 = 0x42;
const EXTENT: [usize; 2] = [4, 3];
const EXPECTED_PIXEL: [u8; 4] = [255, 0, 0, 255];
/// Entry step limits of a locally built program, which runs a C runtime's
/// startup code before its own. The 15 second wall deadline still applies.
const LOCAL_DEFAULT_STEPS: usize = 1_000_000;
const LOCAL_MAX_STEPS: usize = 50_000_000;
/// The largest locally built program that is read.
const LOCAL_MAX_BYTES: u64 = 64 << 20;
/// Exit codes of a local run that are not the guest's own, after `timeout(1)`:
/// the wall deadline, and any other failure of the runner or the guest.
const DEADLINE_CODE: u8 = 124;
const FAILURE_CODE: u8 = 125;
const USAGE: &str = "usage: xenora run --pe <trusted-fixture> --experimental [--trace] [--metal-hud] [--max-steps <1..=10000>]
       xenora run --pe <local-program> --experimental --local-sha256 <64 hex digits> [--trace] [--max-steps <1..=50000000>] [-- <program argument>...]";

struct Options {
    path: PathBuf,
    max_steps: usize,
    worker: bool,
    trace: bool,
    hud: bool,
    /// The SHA-256 the user gave for a program they built themselves. Without
    /// it only the embedded fixtures run.
    local: Option<[u8; 32]>,
    /// What the local program receives after its own name.
    arguments: Vec<String>,
}

fn parse_sha256(text: &str) -> Option<[u8; 32]> {
    let (pairs, rest) = text.as_bytes().as_chunks::<2>();
    if pairs.len() != 32 || !rest.is_empty() {
        return None;
    }
    // Digits only: no sign, no space, and no byte of a longer character.
    let digit = |byte: u8| {
        char::from(byte)
            .to_digit(16)
            .and_then(|value| u8::try_from(value).ok())
    };
    let mut digest = [0; 32];
    for (byte, [high, low]) in digest.iter_mut().zip(pairs) {
        *byte = digit(*high)? << 4 | digit(*low)?;
    }
    Some(digest)
}

impl Options {
    fn parse(args: impl IntoIterator<Item = OsString>) -> Result<Self, String> {
        let mut args = args.into_iter();
        let mut path = None;
        let mut max_steps = None;
        let mut local = None;
        let mut arguments = Vec::new();
        let (mut experimental, mut trace, mut hud, mut worker) = (false, false, false, false);
        while let Some(arg) = args.next() {
            match arg.to_str() {
                Some("--pe") if path.is_none() => {
                    path = Some(PathBuf::from(args.next().ok_or("missing --pe path")?));
                }
                Some("--max-steps") if max_steps.is_none() => {
                    max_steps = Some(
                        args.next()
                            .and_then(|arg| arg.to_str()?.parse::<usize>().ok())
                            .ok_or("--max-steps needs a number")?,
                    );
                }
                Some("--local-sha256") if local.is_none() => {
                    local = Some(
                        args.next()
                            .and_then(|arg| parse_sha256(arg.to_str()?))
                            .ok_or("--local-sha256 needs 64 hexadecimal digits")?,
                    );
                }
                Some("--") => {
                    for argument in args.by_ref() {
                        arguments.push(
                            argument
                                .into_string()
                                .map_err(|_| "program arguments must be UTF-8")?,
                        );
                    }
                }
                Some("--experimental") if !experimental => experimental = true,
                Some("--trace") if !trace => trace = true,
                Some("--metal-hud") if !hud => hud = true,
                Some("--worker") if !worker => worker = true,
                _ => return Err(format!("unknown or repeated run argument {arg:?}")),
            }
        }
        if !experimental {
            return Err("run requires --experimental".into());
        }
        let (default, limit) = if local.is_some() {
            (LOCAL_DEFAULT_STEPS, LOCAL_MAX_STEPS)
        } else {
            (DEFAULT_STEPS, MAX_STEPS)
        };
        let max_steps = max_steps.unwrap_or(default);
        if !(1..=limit).contains(&max_steps) {
            return Err(format!("--max-steps must be in 1..={limit}"));
        }
        if local.is_none() && !arguments.is_empty() {
            return Err("program arguments need --local-sha256".into());
        }
        if local.is_some() && hud {
            return Err("--metal-hud applies to the trusted fixtures only".into());
        }
        Ok(Self {
            path: path.ok_or("missing --pe path")?,
            max_steps,
            worker,
            trace,
            hud,
            local,
            arguments,
        })
    }
}

pub fn command(args: impl IntoIterator<Item = OsString>) -> ExitCode {
    let options = match Options::parse(args) {
        Ok(options) => options,
        Err(error) => {
            eprintln!("{error}\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    let result = match (options.worker, options.local) {
        (true, Some(digest)) => run_local(&options, digest),
        (true, None) => run_worker(&options).map(|()| ExitCode::SUCCESS),
        (false, _) => launch_worker(&options),
    };
    match result {
        Ok(code) => code,
        Err(error) => {
            eprintln!("run failed: {error}");
            if options.local.is_some() {
                ExitCode::from(FAILURE_CODE)
            } else {
                ExitCode::FAILURE
            }
        }
    }
}

fn launch_worker(options: &Options) -> Result<ExitCode, Box<dyn Error>> {
    let started = Instant::now();
    let mut command = Command::new(std::env::current_exe()?);
    command.args(["run", "--experimental", "--worker"]);
    if let Some(digest) = options.local {
        // A local program's own output is the run's output: no read-outs
        // unless they were asked for.
        if options.trace {
            command.arg("--trace");
        }
        command.arg("--local-sha256").arg(hex(digest));
    } else {
        command.args(["--trace", "--metal-hud"]);
    }
    command
        .arg("--max-steps")
        .arg(options.max_steps.to_string())
        .arg("--pe")
        .arg(&options.path);
    if options.local.is_some() {
        command.arg("--").args(&options.arguments);
    }
    let mut child = command.stdin(Stdio::null()).spawn()?;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                return match status.code() {
                    Some(code) => Ok(ExitCode::from(u8::try_from(code)?)),
                    None => Err(format!("worker terminated: {status}").into()),
                };
            }
            Ok(None) if started.elapsed() < WALL_LIMIT => {
                std::thread::sleep(Duration::from_millis(10));
            }
            status => {
                let _ = child.kill();
                child.wait()?;
                status?;
                if options.local.is_some() {
                    eprintln!("run failed: the program exceeded the 15 second wall deadline");
                    return Ok(ExitCode::from(DEADLINE_CODE));
                }
                return Err("experimental worker exceeded its 15 second wall deadline".into());
            }
        }
    }
}

fn verify_fixture(path: &Path) -> Result<&'static [u8], Box<dyn Error>> {
    let valid_metadata =
        |metadata: fs::Metadata| metadata.is_file() && metadata.len() == FIXTURE.len() as u64;
    let rejection =
        "execution denied: input must match an embedded trusted PE fixture byte for byte";
    // Check size before opening and again on the descriptor. Bound the read
    // even if another process changes the file after the metadata checks.
    if !valid_metadata(fs::metadata(path)?) {
        return Err(rejection.into());
    }
    let file = File::open(path)?;
    if !valid_metadata(file.metadata()?) {
        return Err(rejection.into());
    }
    let mut bytes = Vec::with_capacity(FIXTURE.len() + 1);
    file.take(FIXTURE.len() as u64 + 1)
        .read_to_end(&mut bytes)?;
    [FIXTURE, SERVICES_FIXTURE]
        .into_iter()
        .find(|fixture| *fixture == bytes)
        .ok_or_else(|| rejection.into())
}

fn hex(digest: [u8; 32]) -> String {
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn start_deadline() -> std::io::Result<()> {
    std::thread::Builder::new()
        .name("xenora-deadline".into())
        .spawn(|| {
            std::thread::sleep(WALL_LIMIT);
            // The supervisor also enforces this deadline. The worker retains
            // its own deadline if it is invoked directly.
            std::process::exit(i32::from(DEADLINE_CODE));
        })
        .map(drop)
}

/// Read a program the user built themselves and admit it only if its bytes
/// have the SHA-256 the user gave. The bytes that were hashed are the bytes
/// that run, so a change to the file afterwards cannot matter.
fn read_local(path: &Path, expected: [u8; 32]) -> Result<Vec<u8>, Box<dyn Error>> {
    let size = "execution denied: a local program must be a regular file of at most 64 MiB";
    let file = File::open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.len() > LOCAL_MAX_BYTES {
        return Err(size.into());
    }
    let mut bytes = Vec::new();
    file.take(LOCAL_MAX_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > LOCAL_MAX_BYTES {
        return Err(size.into());
    }
    if Sha256::digest(&bytes).as_slice() != expected {
        return Err("execution denied: the file's SHA-256 is not the one given".into());
    }
    Ok(bytes)
}

/// The host exit code for a guest exit status. A host exit code has eight
/// bits and a guest status thirty-two: the low eight are kept, except that a
/// status other than zero never becomes zero.
fn host_exit_code(status: u32) -> u8 {
    match status.to_le_bytes()[0] {
        0 if status != 0 => 1,
        low => low,
    }
}

/// Run a locally built program: no embedded snapshot, no draw service and no
/// expected result. Its arguments, output and exit status are its own.
fn run_local(options: &Options, digest: [u8; 32]) -> Result<ExitCode, Box<dyn Error>> {
    start_deadline()?;
    let bytes = read_local(&options.path, digest)?;
    let name = options
        .path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or("the program's file name must be UTF-8")?;
    if options.trace {
        eprintln!(
            "trace program={name} policy=sha256 entry_step_limit={} wall_limit_ms=15000",
            options.max_steps
        );
    }
    let state = Arc::new(ServicesState::new(Arc::new(SystemClock)));
    let mut arguments = vec![name];
    arguments.extend(options.arguments.iter().map(String::as_str));
    state.set_program_arguments(&arguments)?;
    let mut imports = ImportRegistry::new();
    register_standard_services(&mut imports, Arc::clone(&state));
    let startup = run_synthetic_startup_with_observer(
        &bytes,
        StartupLayout {
            image_base: IMAGE_BASE,
            stack_top: 0x200010000,
            tls_base: 0x300000000,
        },
        imports,
        options.max_steps,
        |event| {
            if options.trace {
                eprintln!("{}", startup_line(&event));
            }
        },
    );
    // Whatever the program wrote is shown, however the run ended.
    let mut stdout = std::io::stdout().lock();
    stdout.write_all(&state.standard_output())?;
    stdout.flush()?;
    std::io::stderr().write_all(&state.standard_error())?;
    let execution = startup?.execution;
    if options.trace {
        eprintln!(
            "trace stage=exit entry_steps={} exit={:?}",
            execution.instructions_executed, execution.exit_reason
        );
    }
    match execution.exit_reason {
        ExitReason::ProcessExit(status) => {
            let code = host_exit_code(status);
            if u32::from(code) != status {
                eprintln!("guest exit status {status} ({status:#x}); host exit code {code}");
            }
            Ok(ExitCode::from(code))
        }
        other => Err(format!(
            "the program stopped without exiting after {} entry steps: {other:?}",
            execution.instructions_executed
        )
        .into()),
    }
}

fn run_worker(options: &Options) -> Result<(), Box<dyn Error>> {
    start_deadline()?;
    let fixture = verify_fixture(&options.path)?;
    let (name, ticket, bound_imports) = if fixture == SERVICES_FIXTURE {
        ("runtime-services", 23, 3)
    } else {
        ("trusted-runner", TICKET, 1)
    };
    if options.trace {
        println!(
            "trace fixture={name} policy=exact-bytes entry_step_limit={} wall_limit_ms=15000",
            options.max_steps
        );
    }

    let capture = Arc::new(Mutex::new(None));
    let draw_capture = Arc::clone(&capture);
    let mut imports = ImportRegistry::new();
    register_standard_services(
        &mut imports,
        Arc::new(ServicesState::new(Arc::new(SystemClock))),
    );
    let trace = options.trace;
    imports.register("xenora.dll", "fixture_draw", move |args, _, memory| {
        let ticket_address = args.get(0, memory).map_err(ExitReason::MemoryFault)?;
        let mut result = draw_capture
            .lock()
            .map_err(|_| ExitReason::IllegalInstruction(0xff))?;
        if result.is_some() {
            return Err(ExitReason::IllegalInstruction(0xff));
        }
        if ticket_address != TICKET_ADDRESS
            || memory
                .read_u64(ticket_address)
                .map_err(ExitReason::MemoryFault)?
                != ticket
        {
            *result = Some(Err(
                "guest draw ticket differed from the expected value".into()
            ));
            return Err(ExitReason::IllegalInstruction(0xff));
        }
        if trace {
            println!("trace import=xenora.dll!fixture_draw tls_ticket={ticket}");
        }
        let frame = draw().map_err(|error| error.to_string());
        let drawn = frame.is_ok();
        *result = Some(frame);
        if drawn {
            Ok(DRAW_RESULT)
        } else {
            Err(ExitReason::IllegalInstruction(0xff))
        }
    });

    let started = Instant::now();
    // Use only the embedded snapshot. The file is solely an exact-byte gate.
    let startup = run_synthetic_startup_with_observer(
        fixture,
        StartupLayout {
            image_base: IMAGE_BASE,
            stack_top: 0x200010000,
            tls_base: 0x300000000,
        },
        imports,
        options.max_steps,
        |event| {
            if options.trace {
                trace_startup(event);
            }
        },
    )?;
    let host_ms = started.elapsed().as_secs_f64() * 1000.0;
    let result = capture
        .lock()
        .map_err(|_| "fixture draw result lock failed")?
        .take();
    let exit = startup.execution.exit_reason;
    let rax = startup.execution.final_state.get(Register::Rax);
    if options.trace {
        println!(
            "trace stage=exit entry_steps={} exit={exit:?} rax={rax:#x}",
            startup.execution.instructions_executed,
        );
    }
    if let Some(Err(error)) = &result {
        return Err(format!("fixture draw failed: {error}").into());
    }
    if exit != ExitReason::Halt {
        return Err(exit.into());
    }
    if startup.relocations.applied_dir64 != 4
        || startup.bound_imports != bound_imports
        || startup.tls_callbacks != 1
        || rax != DRAW_RESULT
    {
        return Err("trusted fixture startup or result differs from the expected values".into());
    }
    let frame = result.ok_or("guest halted without a draw")??;
    if options.trace {
        println!(
            "trace stage=framebuffer extent={}x{} verified_pixels={}",
            EXTENT[0],
            EXTENT[1],
            EXTENT[0] * EXTENT[1],
        );
    }
    if options.hud {
        let gpu_time = frame.gpu_seconds.map_or_else(
            || "unavailable".to_owned(),
            |seconds| format!("{:.3}", seconds * 1000.0),
        );
        println!(
            "Metal HUD | device={} | offscreen={}x{} | draw_calls=1 | verified_pixels={} | host_render_readback_ms={:.3} | host_startup_ms={host_ms:.3} | gpu_command_ms={gpu_time}",
            frame.device,
            EXTENT[0],
            EXTENT[1],
            EXTENT[0] * EXTENT[1],
            frame.host_render_readback_ms,
        );
        let metrics = startup.dbt_metrics;
        println!(
            "DBT | entry_steps={} | native_steps_tls_and_entry={} | blocks_compiled={} | native_bytes={} | native_dispatches={} | service_calls={} | interpreter_fallbacks={}",
            startup.execution.instructions_executed,
            metrics.instructions_executed,
            metrics.blocks_compiled,
            metrics.native_bytes_emitted,
            metrics.dbt_dispatches,
            metrics.service_calls,
            metrics.interpreter_fallbacks,
        );
    }
    println!("Trusted fixture completed one verified offscreen Metal draw.");
    Ok(())
}

fn trace_startup(event: StartupEvent) {
    println!("{}", startup_line(&event));
}

fn startup_line(event: &StartupEvent) -> String {
    match event {
        StartupEvent::Parsed {
            sections,
            entry_rva,
        } => format!("trace stage=parsed sections={sections} entry_rva={entry_rva:#x}"),
        StartupEvent::Mapped { entry_va } => {
            format!("trace stage=mapped base={IMAGE_BASE:#x} entry_va={entry_va:#x}")
        }
        StartupEvent::Relocated { applied_dir64 } => {
            format!("trace stage=relocated dir64_applied={applied_dir64}")
        }
        StartupEvent::ImportsBound { count } => {
            format!("trace stage=imports_bound bound_imports={count}")
        }
        StartupEvent::TlsCompleted {
            callbacks,
            native_steps,
        } => format!(
            "trace stage=tls_completed tls_callbacks={callbacks} native_steps={native_steps}"
        ),
        StartupEvent::Entering { step_limit } => {
            format!("trace stage=entry step_limit={step_limit}")
        }
    }
}

struct Frame {
    device: String,
    host_render_readback_ms: f64,
    gpu_seconds: Option<f64>,
}

fn draw() -> Result<Frame, Box<dyn Error>> {
    let shader = compile_graphics(
        Architecture::Gfx1030,
        include_bytes!("../../../fixtures/rdna2/x9-constant-vs.bin"),
        include_bytes!("../../../fixtures/rdna2/x9-constant-ps.bin"),
    )?;
    let gpu = NativeCompute::new()?;
    let started = Instant::now();
    let (pixels, gpu_seconds) = gpu.render_rgba8_profiled(
        &shader.msl,
        [VERTEX_ENTRY_POINT, PIXEL_ENTRY_POINT],
        &[
            [-1.0, -1.0, 0.5, 1.0],
            [3.0, -1.0, 0.5, 1.0],
            [-1.0, 3.0, 0.5, 1.0],
        ],
        &[[0.0; 4]; 3],
        EXTENT,
        [19, 43, 93, 251],
    )?;
    let host_render_readback_ms = started.elapsed().as_secs_f64() * 1000.0;
    if pixels != EXPECTED_PIXEL.repeat(EXTENT[0] * EXTENT[1]) {
        return Err("offscreen readback did not match opaque red at every pixel".into());
    }
    Ok(Frame {
        device: gpu.device_name().to_owned(),
        host_render_readback_ms,
        gpu_seconds,
    })
}

#[cfg(test)]
mod tests {
    use super::{hex, host_exit_code, parse_sha256};

    #[test]
    fn a_nonzero_status_never_becomes_a_zero_exit_code() {
        // (guest status, host exit code)
        for (status, code) in [
            (0, 0),
            (1, 1),
            (255, 255),
            (256, 1),
            (0x00A1_A1A1, 0xA1),
            (0x8000_0000, 1),
            (u32::MAX, 255),
        ] {
            assert_eq!(host_exit_code(status), code, "{status:#x}");
        }
    }

    #[test]
    fn a_digest_is_sixty_four_hexadecimal_digits() {
        let mut expected = [0; 32];
        for (index, byte) in expected.iter_mut().enumerate() {
            *byte = u8::try_from(index * 8 + 1).unwrap();
        }
        let lower: String = expected.iter().map(|byte| format!("{byte:02x}")).collect();
        assert_eq!(parse_sha256(&lower), Some(expected));
        assert_eq!(parse_sha256(&lower.to_uppercase()), Some(expected));
        assert_eq!(hex(expected), lower);
        for wrong in [
            &lower[..63],
            &format!("{lower}0"),
            &format!("{}g", &lower[..63]),
            &format!("+{}", &lower[..63]),
            &format!(" {}", &lower[..63]),
            // Sixty-four bytes, but not sixty-four digits.
            &format!("{}\u{e9}", &lower[..62]),
            "",
        ] {
            assert_eq!(parse_sha256(wrong), None, "{wrong:?}");
        }
    }
}
