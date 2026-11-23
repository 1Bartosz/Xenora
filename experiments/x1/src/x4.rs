// SPDX-License-Identifier: GPL-3.0-or-later
use crate::cpu::{Fault, FaultKind};
use crate::pe::{self, LoadError};

const IMAGES: [(u8, &[u8]); 2] = [
    (17, include_bytes!("../fixtures/x4-17.exe")),
    (93, include_bytes!("../fixtures/x4-93.exe")),
];

fn decode(address: u64, base: u64, bytes: &[u8; 48]) -> Result<u8, FaultKind> {
    if address != base {
        return Err(FaultKind::InvalidCommand);
    }
    let (command, parameters) = bytes.split_at(32);
    let mut words = [0; 8];
    for (word, bytes) in words.iter_mut().zip(command.as_chunks::<4>().0) {
        *word = u32::from_le_bytes(*bytes);
    }
    let [
        version,
        service,
        target,
        width,
        height,
        format,
        offset,
        length,
    ] = words;
    if service != 3 {
        return Err(FaultKind::UnknownService);
    }
    if (version, target, width, height, format) != (1, 1, 8, 8, 1)
        || length != 4
        || !offset.is_multiple_of(4)
    {
        return Err(FaultKind::InvalidCommand);
    }
    let end = offset
        .checked_add(length)
        .ok_or(FaultKind::InvalidCommand)?;
    let value = parameters
        .get(offset as usize..end as usize)
        .and_then(|bytes| bytes.try_into().ok())
        .map(u32::from_le_bytes)
        .ok_or(FaultKind::InvalidCommand)?;
    u8::try_from(value).map_err(|_| FaultKind::InvalidCommand)
}

fn execute(
    bytes: &[u8],
    mut draw: impl FnMut(u8) -> Result<(), FaultKind>,
) -> Result<([u8; 8], Result<(), Fault>), LoadError> {
    let (mut image, base, read_only) = pe::load_compute(bytes)?;
    let mut state = image.initial(0);
    let mut submitted = false;
    let result = image.run_with_trap(
        &mut state,
        16,
        &mut |state| {
            if submitted {
                return Err(FaultKind::InvalidCommand);
            }
            let parameter = decode(state.gpr[0], base, read_only)?;
            submitted = true;
            draw(parameter)?;
            Ok(0)
        },
        |_| {},
    );
    Ok((image.data, result))
}

#[cfg(feature = "metal")]
pub(crate) fn run() -> std::process::ExitCode {
    use std::process::ExitCode;

    for (expected_parameter, image) in IMAGES {
        let mut readback = None;
        let execution = execute(image, |parameter| {
            println!("X4 accepted p={parameter}");
            readback = Some(crate::graphics::render(parameter).map_err(|error| {
                eprintln!("X4 Metal error {error}");
                FaultKind::DispatchFailed
            })?);
            Ok(())
        });
        let Ok((status, result)) = execution else {
            eprintln!("X4 PE load failed {execution:?}");
            return ExitCode::FAILURE;
        };
        println!("X4 guest status={status:02x?} result={result:x?}");
        let expected: Vec<u8> = (0..8)
            .flat_map(|y| (0..8).flat_map(move |x| [x, y, expected_parameter, 255]))
            .collect();
        let Some(pixels) = readback else {
            return ExitCode::FAILURE;
        };
        if result.is_err() || status != [0; 8] || pixels.as_slice() != expected {
            eprintln!("X4 pixel or completion mismatch for p={expected_parameter}");
            return ExitCode::FAILURE;
        }
        println!("X4 verified 256 bytes p={expected_parameter}");
        print!("X4 rgba8 p={expected_parameter} ");
        for byte in pixels {
            print!("{byte:02x}");
        }
        println!();
    }
    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guest_parameter_reaches_draw() {
        for (expected, image) in IMAGES {
            let mut calls = 0;
            let (status, result) = execute(image, |parameter| {
                calls += 1;
                assert_eq!(parameter, expected);
                Ok(())
            })
            .unwrap();
            assert_eq!(calls, 1);
            assert_eq!(status, [0; 8]);
            result.unwrap();
        }
    }

    #[test]
    fn rejects_invalid_commands_before_draw() {
        for (offset, value) in [
            (0x201, 0x403001u32),
            (0x600, 2),
            (0x604, 2),
            (0x608, 0),
            (0x608, 2),
            (0x60c, 0),
            (0x60c, 9),
            (0x610, u32::MAX),
            (0x614, 0),
            (0x618, 1),
            (0x618, 16),
            (0x618, 0xffff_fffc),
            (0x61c, 0),
            (0x61c, 8),
            (0x624, 256),
        ] {
            let mut image = IMAGES[0].1.to_vec();
            image[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
            let (status, result) = execute(&image, |_| panic!("unexpected draw")).unwrap();
            let kind = if offset == 0x604 {
                FaultKind::UnknownService
            } else {
                FaultKind::InvalidCommand
            };
            assert_eq!(
                result,
                Err(Fault {
                    kind,
                    rip: 0x401005
                })
            );
            assert_eq!(status, [0xa5; 8]);
        }
    }

    #[test]
    #[cfg(feature = "metal")]
    fn failed_render_leaves_guest_status_pending() {
        let (status, result) = execute(IMAGES[0].1, |_| Err(FaultKind::DispatchFailed)).unwrap();
        assert_eq!(
            result,
            Err(Fault {
                kind: FaultKind::DispatchFailed,
                rip: 0x401005
            })
        );
        assert_eq!(status, [0xa5; 8]);
    }
}
