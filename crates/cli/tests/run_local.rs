// SPDX-License-Identifier: GPL-3.0-or-later

//! `xenora run` on a program the user built: the real binary, a real hosted C
//! program, and what a shell sees of it.
//!
//! The programs are the Gate A ones, built from source for each test process
//! by `tests/compat_fixtures/build-gate-a2.sh` and `build-gate-a1.sh`; what
//! they compute is checked by `crates/loader/tests/gate_a2.rs` and
//! `gate_a1.rs`. These tests check the command around them, so every
//! expectation here follows from the command line the test itself wrote: the
//! arguments it passed, the variant it chose and the digest it gave. Digests
//! come from `shasum`, not from the code under test.
//!
//! A missing toolchain is a failure, never a skip.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::OnceLock;

/// The line the Gate A2 program's `atexit` function prints when every check
/// passed. `gate_a2.rs` derives the count.
const ALL_PASSED: &str = "atexit: 76 checks passed\r\n";
/// The exit status of the Gate A1 program, as `gate_a1.rs` has it.
const A1_STATUS: u32 = 0x00A1_A1A1;

fn build(script: &str, folder: &str, lock: &'static OnceLock<PathBuf>) -> &'static Path {
    lock.get_or_init(|| {
        let out = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
            .join(format!("{folder}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&out);
        let script = format!(
            "{}/../../tests/compat_fixtures/{script}",
            env!("CARGO_MANIFEST_DIR")
        );
        let build = Command::new("sh")
            .arg(&script)
            .arg(&out)
            .output()
            .unwrap_or_else(|error| panic!("run {script}: {error}"));
        assert!(
            build.status.success(),
            "{script} failed; the hosted program needs mingw-w64 and the freestanding one \
             a clang and an LLD:\n{}{}",
            String::from_utf8_lossy(&build.stdout),
            String::from_utf8_lossy(&build.stderr)
        );
        out
    })
}

fn hosted(name: &str) -> PathBuf {
    static OUT: OnceLock<PathBuf> = OnceLock::new();
    build("build-gate-a2.sh", "run-local-a2", &OUT).join(format!("{name}.exe"))
}

fn freestanding(name: &str) -> PathBuf {
    static OUT: OnceLock<PathBuf> = OnceLock::new();
    build("build-gate-a1.sh", "run-local-a1", &OUT).join(format!("{name}.exe"))
}

/// The SHA-256 of a file, from the host's `shasum`.
fn digest(path: &Path) -> String {
    let output = Command::new("shasum")
        .args(["-a", "256"])
        .arg(path)
        .output()
        .expect("run shasum");
    assert!(output.status.success(), "shasum {path:?}");
    let text = String::from_utf8(output.stdout).unwrap();
    let digest = text.split_whitespace().next().unwrap().to_owned();
    assert_eq!(digest.len(), 64, "{text:?}");
    digest
}

struct Run {
    code: Option<i32>,
    stdout: String,
    stderr: String,
}

fn xenora(arguments: &[&str]) -> Run {
    let Output {
        status,
        stdout,
        stderr,
    } = Command::new(env!("CARGO_BIN_EXE_xenora"))
        .arg("run")
        .args(arguments)
        .output()
        .expect("run xenora");
    Run {
        code: status.code(),
        stdout: String::from_utf8(stdout).expect("ASCII output"),
        stderr: String::from_utf8(stderr).expect("ASCII output"),
    }
}

/// Run a local program with its own digest and the given program arguments.
fn local(path: &Path, extra: &[&str], program: &[&str]) -> Run {
    let digest = digest(path);
    let mut arguments = vec!["--pe", path.to_str().unwrap(), "--experimental"];
    arguments.extend(["--local-sha256", &digest]);
    arguments.extend(extra);
    arguments.push("--");
    arguments.extend(program);
    xenora(&arguments)
}

/// What the Gate A2 program prints first: its arguments, as it received them.
fn echoed(name: &str, program: &[&str]) -> String {
    let mut text = format!(
        "gate a2: {} arguments\r\n  argv[0] = \"{name}.exe\"\r\n",
        program.len() + 1
    );
    for (index, argument) in program.iter().enumerate() {
        text.push_str(&format!("  argv[{}] = \"{argument}\"\r\n", index + 1));
    }
    text
}

#[test]
fn a_local_program_is_refused_without_the_flag() {
    let path = hosted("gate-a2-O2");
    let run = xenora(&["--pe", path.to_str().unwrap(), "--experimental"]);
    assert_eq!(run.code, Some(1), "{}", run.stderr);
    assert_eq!(run.stdout, "");
    assert!(
        run.stderr
            .contains("execution denied: input must match an embedded trusted PE fixture"),
        "{}",
        run.stderr
    );
}

#[test]
fn a_fixture_with_one_byte_changed_is_refused_without_the_flag() {
    // The same length as an embedded fixture, so only the comparison of the
    // bytes can tell them apart. It is refused before anything is drawn.
    for name in ["trusted-runner", "runtime-services"] {
        let original = format!(
            "{}/../../fixtures/pe/{name}.exe",
            env!("CARGO_MANIFEST_DIR")
        );
        let mut bytes = std::fs::read(&original).unwrap();
        *bytes.last_mut().unwrap() ^= 1;
        let changed = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
            .join(format!("{name}-changed-{}.exe", std::process::id()));
        std::fs::write(&changed, bytes).unwrap();
        let run = xenora(&["--pe", changed.to_str().unwrap(), "--experimental"]);
        std::fs::remove_file(&changed).unwrap();
        assert_eq!(run.code, Some(1), "{name}: {}", run.stderr);
        assert_eq!(run.stdout, "", "{name}");
        assert_eq!(
            run.stderr,
            "run failed: execution denied: input must match an embedded trusted PE fixture byte \
             for byte\n",
            "{name}"
        );
    }
}

#[test]
fn a_local_program_runs_with_its_arguments_output_and_exit_status() {
    for level in ["O0", "O1", "O2", "Os"] {
        let name = format!("gate-a2-{level}");
        let program = ["2026", "xenora"];
        let run = local(&hosted(&name), &[], &program);
        assert_eq!(run.code, Some(0), "{name}: {}", run.stderr);
        assert_eq!(run.stderr, "", "{name}");
        assert!(
            run.stdout.starts_with(&echoed(&name, &program)),
            "{name}: {:?}",
            run.stdout
        );
        assert!(run.stdout.ends_with(ALL_PASSED), "{name}: {:?}", run.stdout);
    }
}

#[test]
fn program_arguments_arrive_as_they_were_written() {
    // A space, an empty-looking option, the separator itself and words that
    // are options of `xenora run`: after `--` they are the program's.
    for label in ["two words", "--trace", "--", "--local-sha256", "a\"b"] {
        let program = ["7", label];
        let run = local(&hosted("gate-a2-O1"), &[], &program);
        // The program prints a quote in an argument as it is.
        assert!(
            run.stdout.starts_with(&echoed("gate-a2-O1", &program)),
            "{label:?}: {:?}",
            run.stdout
        );
        assert_eq!(run.code, Some(0), "{label:?}: {}", run.stderr);
        assert!(run.stdout.ends_with(ALL_PASSED), "{label:?}");
    }
}

#[test]
fn the_exit_status_is_the_programs_own() {
    // (variant, program arguments, exit status, standard error)
    let cases: [(&str, &[&str], i32, &str); 6] = [
        ("gate-a2-exit", &["2026", "xenora"], 42, ""),
        ("gate-a2-abort", &["2026", "xenora"], 3, ""),
        (
            "gate-a2-break",
            &["2026", "xenora"],
            22,
            "gate a2: check 22 failed\r\n",
        ),
        // The program's own checks on its arguments: too few, a seed that is
        // not a number, and an empty label, which must arrive empty.
        ("gate-a2-O1", &[], 11, "gate a2: check 11 failed\r\n"),
        (
            "gate-a2-O1",
            &["5x", "label"],
            13,
            "gate a2: check 13 failed\r\n",
        ),
        ("gate-a2-O1", &["5", ""], 14, "gate a2: check 14 failed\r\n"),
    ];
    for (name, program, status, errors) in cases {
        let run = local(&hosted(name), &[], program);
        let what = format!("{name} {program:?}");
        assert_eq!(run.code, Some(status), "{what}: {}", run.stderr);
        assert_eq!(run.stderr, errors, "{what}");
        assert!(
            run.stdout.starts_with(&echoed(name, program)),
            "{what}: {:?}",
            run.stdout
        );
        assert!(!run.stdout.contains("76 checks passed"), "{what}");
    }
}

#[test]
fn a_status_wider_than_eight_bits_is_reported_in_full() {
    // A host exit code has eight bits. The Gate A1 program exits with a
    // 32-bit status and prints nothing.
    let run = local(&freestanding("gate-a1-O2"), &[], &[]);
    assert_eq!(run.code, Some(0xA1), "{}", run.stderr);
    assert_eq!(run.stdout, "");
    assert_eq!(
        run.stderr,
        format!("guest exit status {A1_STATUS} ({A1_STATUS:#x}); host exit code 161\n")
    );
}

#[test]
fn a_wrong_digest_runs_nothing() {
    let path = hosted("gate-a2-O2");
    let own = digest(&path);
    // Another file's digest, one digit off, and a file changed after it was
    // hashed.
    let other = digest(&hosted("gate-a2-O1"));
    let last = if own.ends_with('0') { "1" } else { "0" };
    let near = format!("{}{last}", &own[..63]);
    let changed = path.with_file_name("gate-a2-changed.exe");
    let mut bytes = std::fs::read(&path).unwrap();
    *bytes.last_mut().unwrap() ^= 1;
    std::fs::write(&changed, bytes).unwrap();
    for (file, given) in [(&path, &other), (&path, &near), (&changed, &own)] {
        let run = xenora(&[
            "--pe",
            file.to_str().unwrap(),
            "--experimental",
            "--local-sha256",
            given,
            "--",
            "2026",
            "xenora",
        ]);
        assert_eq!(run.code, Some(125), "{file:?}: {}", run.stderr);
        assert_eq!(run.stdout, "", "{file:?}");
        assert_eq!(
            run.stderr, "run failed: execution denied: the file's SHA-256 is not the one given\n",
            "{file:?}"
        );
    }
    // The digest is not case sensitive, and the right one still runs.
    let run = xenora(&[
        "--pe",
        path.to_str().unwrap(),
        "--experimental",
        "--local-sha256",
        &own.to_uppercase(),
        "--",
        "2026",
        "xenora",
    ]);
    assert_eq!(run.code, Some(0), "{}", run.stderr);
}

#[test]
fn a_step_limit_that_is_too_small_is_a_failure_not_an_exit() {
    let path = hosted("gate-a2-O0");
    let program = ["2026", "xenora"];
    let whole = local(&path, &[], &program);
    assert_eq!(whole.code, Some(0), "{}", whole.stderr);
    let run = local(&path, &["--max-steps", "5000"], &program);
    assert_eq!(run.code, Some(125), "{}", run.stderr);
    assert_eq!(
        run.stderr,
        "run failed: the program stopped without exiting after 5000 entry steps: \
         StepLimitExceeded\n"
    );
    // What it had written by then is shown, and is the start of the whole.
    assert!(run.stdout.len() < whole.stdout.len());
    assert!(whole.stdout.starts_with(&run.stdout), "{:?}", run.stdout);
    assert!(!run.stdout.contains("checks passed"));
}

#[test]
fn the_trace_goes_to_standard_error_and_leaves_the_output_alone() {
    let path = hosted("gate-a2-O2");
    let program = ["2026", "xenora"];
    let plain = local(&path, &[], &program);
    let traced = local(&path, &["--trace"], &program);
    assert_eq!(traced.code, Some(0), "{}", traced.stderr);
    assert_eq!(traced.stdout, plain.stdout);
    assert!(
        traced
            .stderr
            .starts_with("trace program=gate-a2-O2.exe policy=sha256 entry_step_limit=1000000 "),
        "{}",
        traced.stderr
    );
    assert!(
        traced.stderr.contains("exit=ProcessExit(0)"),
        "{}",
        traced.stderr
    );
}

#[test]
fn bad_command_lines_are_usage_errors() {
    let path = hosted("gate-a2-O2");
    let path = path.to_str().unwrap();
    let own = digest(Path::new(path));
    let short = &own[..63];
    let not_hex = format!("{}g", &own[..63]);
    let signed = format!("+{}", &own[..63]);
    let cases: [&[&str]; 8] = [
        // Program arguments without the opt-in.
        &["--pe", path, "--experimental", "--", "2026", "xenora"],
        &["--pe", path, "--experimental", "--local-sha256", short],
        &["--pe", path, "--experimental", "--local-sha256", &not_hex],
        &["--pe", path, "--experimental", "--local-sha256", &signed],
        &["--pe", path, "--experimental", "--local-sha256"],
        &["--pe", path, "--local-sha256", &own],
        &[
            "--pe",
            path,
            "--experimental",
            "--local-sha256",
            &own,
            "--metal-hud",
        ],
        &[
            "--pe",
            path,
            "--experimental",
            "--local-sha256",
            &own,
            "--max-steps",
            "50000001",
        ],
    ];
    for arguments in cases {
        let run = xenora(arguments);
        assert_eq!(run.code, Some(2), "{arguments:?}: {}", run.stderr);
        assert_eq!(run.stdout, "", "{arguments:?}");
        assert!(run.stderr.contains("usage: xenora run"), "{arguments:?}");
    }
    // The fixtures' own step limit is what it was.
    let run = xenora(&["--pe", path, "--experimental", "--max-steps", "10001"]);
    assert_eq!(run.code, Some(2));
    assert!(
        run.stderr.contains("--max-steps must be in 1..=10000"),
        "{}",
        run.stderr
    );
}
