// SPDX-License-Identifier: GPL-3.0-or-later

//! Running a case on the reference and on Metal. The helpers mirror the ones in
//! `known_answers.rs` (copied, not shared, so that file stays untouched).

use std::collections::{BTreeMap, BTreeSet, HashMap};

use xenora_shader::{Architecture, CompileError, ENTRY_POINT, compile};

/// One program, its lane input and the guide's expected lane output.
pub struct Case {
    pub form: &'static str,
    pub family: &'static str,
    pub label: String,
    pub code: Vec<u8>,
    pub input: Vec<u32>,
    pub expected: Vec<u32>,
}

/// A program every implementation must refuse, at byte `offset`: as truncated
/// when `truncated`, else as outside the supported subset.
pub struct Rejection {
    pub family: &'static str,
    pub truncated: bool,
    pub label: String,
    pub code: Vec<u8>,
    pub offset: usize,
}

pub fn words(code: &[u32]) -> Vec<u8> {
    code.iter().copied().flat_map(u32::to_le_bytes).collect()
}

/// The cases that failed, grouped by family: the count and the first message.
#[derive(Default)]
pub struct Failures(BTreeMap<&'static str, (usize, String)>);

impl Failures {
    fn record(&mut self, family: &'static str, message: String) {
        let entry = self.0.entry(family).or_insert((0, message));
        entry.0 += 1;
    }

    /// Panics with one line per failing family.
    pub fn assert_none(&self, what: &str) {
        let lines: Vec<String> = self
            .0
            .iter()
            .map(|(family, (n, first))| format!("  {family}: {n} failed, first: {first}"))
            .collect();
        assert!(lines.is_empty(), "{what}:\n{}", lines.join("\n"));
    }
}

pub fn check_reference(cases: &[Case]) -> Failures {
    let mut failures = Failures::default();
    for case in cases {
        let compiled = compile(Architecture::Gfx1030, &case.code);
        let reference = xenora_ref::execute(&case.code, &case.input);
        if compiled.is_err() {
            failures.record(case.family, format!("{}: {compiled:?}", case.label));
        } else if reference.as_deref().ok() != Some(case.expected.as_slice()) {
            let expected = &case.expected;
            let message = format!("{}: {reference:?}, expected {expected:?}", case.label);
            failures.record(case.family, message);
        }
    }
    failures
}

pub fn check_rejected(rejections: &[Rejection]) -> Failures {
    let mut failures = Failures::default();
    for rejection in rejections {
        if let Err(message) = rejected_at(rejection) {
            failures.record(rejection.family, format!("{}: {message}", rejection.label));
        }
    }
    failures
}

fn rejected_at(rejection: &Rejection) -> Result<(), String> {
    let (compiler, compiler_truncated) = match compile(Architecture::Gfx1030, &rejection.code) {
        Ok(_) => return Err("the compiler accepted it".into()),
        Err(CompileError::Verify { offset, .. }) => (offset, false),
        // The decoder crate is not a dependency of this package, so its
        // `DecodeErrorKind::Truncated` variant is recognised by name.
        Err(CompileError::Decode(decode)) => (
            decode.offset,
            format!("{:?}", decode.kind).starts_with("Truncated "),
        ),
        Err(other) => return Err(format!("{other:?}")),
    };
    let reference = match xenora_ref::execute(&rejection.code, &[0, 1]) {
        Ok(_) => return Err("the reference accepted it".into()),
        Err(error) => error,
    };
    // The reference names a missing word "truncated ..."; its fall-through
    // "unsupported or truncated ..." does not say which, and counts as unsupported.
    let reference_truncated = reference.detail.starts_with("truncated");
    if compiler != rejection.offset || reference.offset != rejection.offset {
        return Err(format!(
            "compiler at {compiler:#x}, reference at {:#x}, expected {:#x}",
            reference.offset, rejection.offset
        ));
    }
    if compiler_truncated != rejection.truncated || reference_truncated != rejection.truncated {
        return Err(format!(
            "truncated: compiler {compiler_truncated}, reference {reference_truncated} ({}), \
             expected {}",
            reference.detail, rejection.truncated
        ));
    }
    Ok(())
}

/// Runs every case on Metal. Cases whose translated MSL and input are identical
/// are one program, so each distinct program runs once and its output is
/// compared with every case that shares it. The programs are spread over
/// `THREADS` threads, each with its own device queue, because pipeline
/// compilation dominates the run time. Returns the failures and the number of
/// programs.
pub fn check_metal(cases: &[Case]) -> (Failures, usize) {
    const THREADS: usize = 4;
    let mut programs = BTreeSet::new();
    let mut failures = Failures::default();
    let mut keys = Vec::with_capacity(cases.len());
    for case in cases {
        match compile(Architecture::Gfx1030, &case.code) {
            Ok(compiled) => {
                let key = (compiled.msl, case.input.clone());
                programs.insert(key.clone());
                keys.push(Some(key));
            }
            Err(error) => {
                failures.record(case.family, format!("{}: {error:?}", case.label));
                keys.push(None);
            }
        }
    }
    let work: Vec<&(String, Vec<u32>)> = programs.iter().collect();
    let chunk = work.len().div_ceil(THREADS).max(1);
    let outputs: HashMap<(String, Vec<u32>), Result<Vec<u32>, String>> = std::thread::scope(|s| {
        let handles: Vec<_> = work
            .chunks(chunk)
            .map(|part| {
                s.spawn(move || {
                    let metal = xenora_metal::NativeCompute::new().unwrap();
                    part.iter()
                        .map(|&key| {
                            let output = metal
                                .run_words(&key.0, ENTRY_POINT, &key.1)
                                .map_err(|error| error.to_string());
                            (key.clone(), output)
                        })
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        handles
            .into_iter()
            .flat_map(|handle| handle.join().unwrap())
            .collect()
    });
    for (case, key) in cases.iter().zip(keys) {
        let Some(key) = key else { continue };
        match &outputs[&key] {
            Ok(output) if *output == case.expected => {}
            Ok(output) => {
                let expected = &case.expected;
                let message = format!("{}: {output:?}, expected {expected:?}", case.label);
                failures.record(case.family, message);
            }
            Err(error) => failures.record(case.family, format!("{}: {error}", case.label)),
        }
    }
    (failures, outputs.len())
}

/// The number of cases in each family, which `expected` lists in full.
pub fn assert_counts<'a>(
    cases: &[Case],
    expected: impl Iterator<Item = &'a (&'static str, usize)>,
) {
    let expected: BTreeMap<_, _> = expected.map(|&(family, n)| (family, n)).collect();
    assert_eq!(tally(cases, |case| case.family), expected);
}

pub fn print_counts(cases: &[Case]) {
    for (name, n) in tally(cases, |case| case.form) {
        eprintln!("  form {name}: {n}");
    }
    for (name, n) in tally(cases, |case| case.family) {
        eprintln!("  family {name}: {n}");
    }
}

fn tally(cases: &[Case], key: fn(&Case) -> &'static str) -> BTreeMap<&'static str, usize> {
    let mut counts = BTreeMap::new();
    for case in cases {
        *counts.entry(key(case)).or_default() += 1;
    }
    counts
}
