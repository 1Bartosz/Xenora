// SPDX-License-Identifier: GPL-3.0-or-later
use crate::cpu::{Fault, FaultKind};
use crate::pe::{self, LoadError};

const IMAGE: &[u8] = include_bytes!("../fixtures/x3.exe");
const COMMAND_LEN: usize = 32;
const MAX_WORDS: usize = 4;

#[derive(Debug, PartialEq, Eq)]
struct OrDispatch {
    input: Vec<u32>,
}

fn decode(
    address: u64,
    command_base: u64,
    command: &[u8],
    input: &[u8],
) -> Result<OrDispatch, FaultKind> {
    let offset = address
        .checked_sub(command_base)
        .and_then(|value| usize::try_from(value).ok())
        .ok_or(FaultKind::InvalidCommand)?;
    let end = offset
        .checked_add(COMMAND_LEN)
        .ok_or(FaultKind::InvalidCommand)?;
    let bytes: &[u8; COMMAND_LEN] = command
        .get(offset..end)
        .and_then(|bytes| bytes.try_into().ok())
        .ok_or(FaultKind::InvalidCommand)?;
    let word = |index: usize| {
        let start = index * 4;
        u32::from_le_bytes([
            bytes[start],
            bytes[start + 1],
            bytes[start + 2],
            bytes[start + 3],
        ])
    };

    if word(1) != 2 {
        return Err(FaultKind::UnknownService);
    }
    // Version 1, OR kernel, single input handle, read-only access.
    if (word(0), word(2), word(3), word(4)) != (1, 1, 1, 1) {
        return Err(FaultKind::InvalidCommand);
    }
    let start = word(5) as usize;
    let length = word(6) as usize;
    let count = word(7) as usize;
    if !(1..=MAX_WORDS).contains(&count) || length != count * 4 || !start.is_multiple_of(4) {
        return Err(FaultKind::InvalidCommand);
    }
    let end = start.checked_add(length).ok_or(FaultKind::InvalidCommand)?;
    let input = input.get(start..end).ok_or(FaultKind::InvalidCommand)?;
    Ok(OrDispatch {
        input: input
            .as_chunks::<4>()
            .0
            .iter()
            .copied()
            .map(u32::from_le_bytes)
            .collect(),
    })
}

fn execute(
    bytes: &[u8],
    mut dispatch: impl FnMut(OrDispatch) -> Result<(), FaultKind>,
) -> Result<([u8; 8], Result<(), Fault>), LoadError> {
    let (mut image, command_base, read_only) = pe::load_compute(bytes)?;
    let (command, input) = read_only.split_at(COMMAND_LEN);
    let mut state = image.initial(0);
    let mut submitted = false;
    let result = image.run_with_trap(
        &mut state,
        16,
        &mut |state| {
            if submitted {
                return Err(FaultKind::InvalidCommand);
            }
            let request = decode(state.gpr[0], command_base, command, input)?;
            submitted = true;
            dispatch(request)?;
            Ok(0)
        },
        |_| {},
    );
    Ok((image.data, result))
}

#[cfg(feature = "metal")]
pub(crate) fn run() -> std::process::ExitCode {
    const EXPECTED: [u32; 3] = [0x00ff_00ff, 0x12ff_56ff, 0xffff_ffff];
    let mut readback = None;
    let execution = execute(IMAGE, |request| {
        println!("X3 accepted submit input={:08x?}", request.input);
        let output = xenora_metal::NativeCompute::new()
            .and_then(|metal| {
                println!("X3 device={}", metal.device_name());
                metal.run_words(include_str!("x3.metal"), "x3_or", &request.input)
            })
            .map_err(|error| {
                eprintln!("X3 Metal error {error}");
                FaultKind::DispatchFailed
            })?;
        println!("X3 completed readback={output:08x?} expected={EXPECTED:08x?}");
        readback = Some(output);
        Ok(())
    });
    let (status, result) = match execution {
        Ok(execution) => execution,
        Err(error) => {
            eprintln!("X3 PE load failed: {error:?}");
            return std::process::ExitCode::FAILURE;
        }
    };
    println!(
        "X3 guest status={:016x} result={result:x?}",
        u64::from_le_bytes(status)
    );
    if result.is_ok() && status == [0; 8] && readback.as_deref() == Some(EXPECTED.as_slice()) {
        std::process::ExitCode::SUCCESS
    } else {
        std::process::ExitCode::FAILURE
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PENDING: [u8; 8] = [0xa5; 8];

    #[test]
    fn guest_command_decodes_the_bounded_subrange() {
        let mut requests = Vec::new();
        let (status, result) = execute(IMAGE, |request| {
            requests.push(request);
            Ok(())
        })
        .unwrap();
        result.unwrap();
        assert_eq!(
            requests,
            [OrDispatch {
                input: vec![0, 0x1234_5678, 0xffff_ffff]
            }]
        );
        assert_eq!(status, [0; 8]);
    }

    #[test]
    #[cfg(feature = "metal")]
    fn failed_dispatch_faults_before_guest_status_store() {
        let mut calls = 0;
        let (status, result) = execute(IMAGE, |request| {
            calls += 1;
            assert_eq!(request.input, [0, 0x1234_5678, 0xffff_ffff]);
            Err(FaultKind::DispatchFailed)
        })
        .unwrap();
        let fault = result.unwrap_err();
        assert_eq!(calls, 1);
        assert_eq!(
            fault,
            Fault {
                kind: FaultKind::DispatchFailed,
                rip: 0x401005,
            }
        );
        assert_eq!(status, [0xa5; 8]);
    }

    #[test]
    fn malformed_guest_commands_fault_before_dispatch() {
        for (field, value, kind) in [
            (0, 2u32, FaultKind::InvalidCommand),
            (1, 99, FaultKind::UnknownService),
            (2, 2, FaultKind::InvalidCommand),
            (3, 0, FaultKind::InvalidCommand),
            (4, 2, FaultKind::InvalidCommand),
            (5, 1, FaultKind::InvalidCommand),
            (5, 8, FaultKind::InvalidCommand),
            (5, 0xffff_fffc, FaultKind::InvalidCommand),
            (6, 16, FaultKind::InvalidCommand),
            (7, 0, FaultKind::InvalidCommand),
            (7, 5, FaultKind::InvalidCommand),
        ] {
            let mut image = IMAGE.to_vec();
            let start = 0x600 + field * 4;
            image[start..start + 4].copy_from_slice(&value.to_le_bytes());
            assert_rejected(&image, kind);
        }
        for address in [0x401000u32, 0x402fff, 0x403001, 0xffff_ffff] {
            let mut image = IMAGE.to_vec();
            image[0x201..0x205].copy_from_slice(&address.to_le_bytes());
            assert_rejected(&image, FaultKind::InvalidCommand);
        }
    }

    #[test]
    fn pe_entry_and_trap_bytes_control_submission() {
        let mut image = IMAGE.to_vec();
        image[0xa9] = 0x20; // Entry points into writable .data.
        assert_eq!(
            execute(&image, |_| panic!("unexpected dispatch")),
            Err(LoadError::Entry)
        );

        image[0xa9] = IMAGE[0xa9];
        image[0x205] = 0xcc; // Unsupported instruction in place of UD2.
        assert_rejected(&image, FaultKind::Unsupported);
    }

    #[test]
    fn pe_sections_supply_input_and_initial_status() {
        let mut image = IMAGE.to_vec();
        image[0x400] = 0x5a;
        image[0x624..0x628].copy_from_slice(&42u32.to_le_bytes());
        let (status, result) = execute(&image, |request| {
            assert_eq!(request.input, [42, 0x1234_5678, 0xffff_ffff]);
            Err(FaultKind::InvalidCommand)
        })
        .unwrap();
        assert_eq!(result.unwrap_err().rip, 0x401005);
        assert_eq!(status, [0x5a, 0xa5, 0xa5, 0xa5, 0xa5, 0xa5, 0xa5, 0xa5]);

        image[0x1ff] = 0xc0; // Writable .rdata.
        assert_eq!(
            execute(&image, |_| panic!("unexpected dispatch")),
            Err(LoadError::Section)
        );
    }

    fn assert_rejected(image: &[u8], kind: FaultKind) {
        let (status, result) = execute(image, |_| panic!("unexpected dispatch")).unwrap();
        let fault = result.unwrap_err();
        assert_eq!(
            fault,
            Fault {
                kind,
                rip: 0x401005
            }
        );
        assert_eq!(status, PENDING);
        println!(
            "X3 rejected {fault:x?} guest status={:016x}",
            u64::from_le_bytes(status)
        );
    }
}
