// SPDX-License-Identifier: GPL-3.0-or-later

use std::error::Error;
use std::process::{Command, ExitCode};
use std::time::{Duration, Instant};
use std::{env, thread};

use xenora_metal::NativeCompute;
use xenora_shader::{Architecture, ENTRY_POINT, compile};

const INPUT: [u32; 6] = [0, 1, 0x0040_0000, 0x00ff_00ff, 0x8000_0000, u32::MAX];
const ADD: &[u8] = include_bytes!("../../fixtures/rdna2/add.bin");
const BRANCH: &[u8] = include_bytes!("../../fixtures/rdna2/bitwise-branch.bin");
const FLOAT: &[u8] = include_bytes!("../../fixtures/rdna2/float-exact.bin");
const ADD_MSL: &str = include_str!("../../fixtures/golden/add.metal");
const BRANCH_MSL: &str = include_str!("../../fixtures/golden/bitwise-branch.metal");
const FLOAT_MSL: &str = include_str!("../../fixtures/golden/float-exact.metal");
const DEADLINE: Duration = Duration::from_secs(30);
const SURVIVED: u8 = 2;
const VALIDATION: [(&str, &str); 4] = [
    ("MTL_DEBUG_LAYER", "1"),
    ("MTL_SHADER_VALIDATION", "1"),
    ("MTL_SHADER_VALIDATION_REPORT_TO_STDERR", "1"),
    ("MTL_SHADER_VALIDATION_ABORT_ON_FAULT", "1"),
];

#[derive(Clone, Copy)]
enum Fixture {
    Add,
    AddNegative,
    BranchTaken,
    BranchFallthrough,
    BranchRetargeted,
    Float,
}

impl Fixture {
    fn code(self) -> Vec<u8> {
        match self {
            Self::Add => ADD.to_vec(),
            Self::AddNegative => {
                let mut code = ADD.to_vec();
                // Replace the eight-byte move of the literal with v_mov_b32 v2, -1.
                assert_eq!(&code[12..20], &[0xff, 2, 4, 0x7e, 0x78, 0x56, 0x34, 0x12]);
                code.splice(12..20, 0x7e04_02c1_u32.to_le_bytes());
                code
            }
            Self::BranchTaken | Self::BranchFallthrough | Self::BranchRetargeted => {
                let mut code = BRANCH.to_vec();
                if matches!(self, Self::BranchFallthrough | Self::BranchRetargeted) {
                    // Compare s8=7 with 7 so SCC=1 and s_cbranch_scc0 is not taken.
                    assert_eq!(code[17], 0x86);
                    code[17] = 0x87;
                }
                if matches!(self, Self::BranchRetargeted) {
                    // Change the unconditional target from .Ldone to the next instruction.
                    assert_eq!(&code[28..32], &0xbf82_0007_u32.to_le_bytes());
                    code[28] = 0;
                }
                code
            }
            Self::Float => FLOAT.to_vec(),
        }
    }

    fn expected(self) -> [u32; INPUT.len()] {
        match self {
            Self::Add => INPUT.map(|word| (u64::from(word) + 0x1234_5678) as u32),
            Self::AddNegative => INPUT.map(|word| (u64::from(word) + 0xffff_ffff) as u32),
            Self::BranchTaken | Self::BranchRetargeted => {
                INPUT.map(|word| (word & 0x00ff_00ff) ^ 0x1234_5678)
            }
            Self::BranchFallthrough => [0; INPUT.len()],
            Self::Float => INPUT.map(|word| {
                // Bit 22 selects 1 or 3/2. Adding and then multiplying by 5/2
                // produces 35/4 (0x410c0000) or 10 (0x41200000).
                if word & 0x0040_0000 == 0 {
                    0x410c_0000
                } else {
                    0x4120_0000
                }
            }),
        }
    }

    fn reviewed_msl(self) -> String {
        match self {
            Self::Add => ADD_MSL.into(),
            Self::AddNegative => ADD_MSL.replace("0x12345678u", "0xffffffffu"),
            Self::BranchTaken => BRANCH_MSL.into(),
            Self::BranchFallthrough | Self::BranchRetargeted => {
                let (header, _) = BRANCH_MSL
                    .split_once("    uint t0 = input[index];")
                    .unwrap();
                let mut msl =
                    format!("{header}    uint t0 = input[index];\n    uint t2 = 0x00000000u;\n");
                if matches!(self, Self::BranchFallthrough) {
                    msl.push_str("    output[index] = t2;\n");
                } else {
                    msl.push_str(concat!(
                        "    uint t4 = 0x00ff00ffu;\n",
                        "    uint t5 = t0 & t4;\n",
                        "    uint t6 = 0x12345678u;\n",
                        "    uint t7 = t5 ^ t6;\n",
                        "    output[index] = t7;\n",
                    ));
                }
                msl.push_str("}\n");
                msl
            }
            Self::Float => FLOAT_MSL.into(),
        }
    }

    fn checked_msl(self, code: &[u8]) -> Result<String, Box<dyn Error>> {
        let msl = compile(Architecture::Gfx1030, code)?.msl;
        // Never dispatch a changed kernel shape until this test's allow-list is reviewed.
        if msl != self.reviewed_msl() {
            return Err(format!("generated MSL differs from the reviewed kernel\n{msl}").into());
        }
        Ok(msl)
    }
}

enum Mutation {
    Replace(&'static str, &'static str),
    SelectPath(Fixture),
}

struct Case {
    name: &'static str,
    baseline: Fixture,
    mutation: Mutation,
}

use Fixture::{Add, AddNegative, BranchFallthrough, BranchRetargeted, BranchTaken, Float};
use Mutation::{Replace, SelectPath};

const CASES: &[Case] = &[
    Case {
        name: "load-zero-instead-of-lane-word",
        baseline: Add,
        mutation: Replace("uint t0 = input[index];", "uint t0 = 0u;"),
    },
    Case {
        name: "literal-immediate-off-by-one",
        baseline: Add,
        mutation: Replace("0x12345678u", "0x12345679u"),
    },
    Case {
        name: "inline-negative-immediate-zero-extended",
        baseline: AddNegative,
        mutation: Replace("0xffffffffu", "0x000000ffu"),
    },
    Case {
        name: "unsigned-add-emitted-as-subtract",
        baseline: Add,
        mutation: Replace("t0 + t1", "t0 - t1"),
    },
    Case {
        name: "unsigned-add-selects-wrong-source",
        baseline: Add,
        mutation: Replace("t0 + t1", "t0 + t0"),
    },
    Case {
        name: "store-selects-wrong-value",
        baseline: Add,
        mutation: Replace("output[index] = t2;", "output[index] = t1;"),
    },
    Case {
        name: "bitwise-and-emitted-as-or",
        baseline: BranchTaken,
        mutation: Replace("t0 & t2", "t0 | t2"),
    },
    Case {
        name: "bitwise-xor-emitted-as-and",
        baseline: BranchTaken,
        mutation: Replace("t3 ^ t4", "t3 & t4"),
    },
    Case {
        name: "incorrect-scc-path",
        baseline: BranchTaken,
        mutation: SelectPath(BranchFallthrough),
    },
    Case {
        name: "incorrect-unconditional-branch-target",
        baseline: BranchFallthrough,
        mutation: SelectPath(BranchRetargeted),
    },
    Case {
        name: "float-add-emitted-as-multiply",
        baseline: Float,
        mutation: Replace(
            "as_type<float>(t4) + as_type<float>(t5)",
            "as_type<float>(t4) * as_type<float>(t5)",
        ),
    },
    Case {
        name: "float-multiply-emitted-as-add",
        baseline: Float,
        mutation: Replace(
            "as_type<float>(t6) * as_type<float>(t5)",
            "as_type<float>(t6) + as_type<float>(t5)",
        ),
    },
];

fn exercise(case: &Case) -> Result<ExitCode, Box<dyn Error>> {
    if VALIDATION
        .iter()
        .any(|&(name, value)| env::var(name).as_deref() != Ok(value))
    {
        return Err("the child process requires Metal API and shader validation".into());
    }
    let code = case.baseline.code();
    let expected = case.baseline.expected();
    let baseline = case.baseline.checked_msl(&code)?;
    let reference = xenora_ref::execute(&code, &INPUT)?;
    if reference != expected {
        return Err(format!("reference {reference:08x?} did not match {expected:08x?}").into());
    }
    let mutated = match case.mutation {
        Replace(before, after) => {
            if baseline.matches(before).count() != 1 {
                return Err(format!("expected exactly one mutation site {before:?}").into());
            }
            baseline.replacen(before, after, 1)
        }
        SelectPath(fixture) => fixture.checked_msl(&fixture.code())?,
    };
    if baseline == mutated {
        return Err("mutation did not change the MSL".into());
    }

    // All checked kernels are straight-line. The mutations only replace values,
    // operators or the selected straight-line path. Addresses never change.
    let device = NativeCompute::new()?;
    println!(
        "device={} api_validation=1 shader_validation=1",
        device.device_name()
    );
    let actual = device.run_words(&baseline, ENTRY_POINT, &INPUT)?;
    if actual != expected {
        return Err(format!("baseline GPU {actual:08x?} did not match {expected:08x?}").into());
    }
    let mutant = device.run_words(&mutated, ENTRY_POINT, &INPUT)?;
    if mutant.len() != expected.len() {
        return Err("mutant readback had the wrong word count".into());
    }
    println!("baseline={expected:08x?} mutant={mutant:08x?}");
    Ok(if mutant == expected {
        ExitCode::from(SURVIVED)
    } else {
        ExitCode::SUCCESS
    })
}

fn campaign() -> Result<ExitCode, Box<dyn Error>> {
    let executable = env::current_exe()?;
    let mut killed = 0;
    let mut survived = 0;
    for case in CASES {
        println!("case={}", case.name);
        let mut child = Command::new(&executable)
            .args(["--child", case.name])
            .envs(VALIDATION)
            .spawn()?;
        let deadline = Instant::now() + DEADLINE;
        loop {
            if let Some(status) = child.try_wait()? {
                match status.code() {
                    Some(0) => {
                        killed += 1;
                        println!("result=killed");
                    }
                    Some(code) if code == i32::from(SURVIVED) => {
                        survived += 1;
                        println!("result=survived");
                    }
                    _ => {
                        return Err(format!(
                            "result=error case={} status={status} killed={killed} survived={survived}",
                            case.name
                        ).into());
                    }
                }
                break;
            }
            if Instant::now() >= deadline {
                let killed_child = child.kill();
                child.wait()?;
                killed_child?;
                return Err(format!(
                    "result=timeout case={} deadline={DEADLINE:?} killed={killed} survived={survived}",
                    case.name
                ).into());
            }
            thread::sleep(Duration::from_millis(10));
        }
    }
    println!(
        "generated={} killed={killed} survived={survived}",
        CASES.len()
    );
    Ok(if survived == 0 {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(SURVIVED)
    })
}

fn main() -> Result<ExitCode, Box<dyn Error>> {
    let args: Vec<_> = env::args().skip(1).collect();
    match args.as_slice() {
        [] => campaign(),
        [child, name] if child == "--child" => {
            let case = CASES
                .iter()
                .find(|case| case.name == name)
                .ok_or("unknown case")?;
            exercise(case)
        }
        _ => Err("unexpected arguments".into()),
    }
}
