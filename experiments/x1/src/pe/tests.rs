// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;

#[test]
fn exact_trace() {
    let mut image = load(FIXTURE).unwrap();
    let mut state = image.initial(41);
    let initial = State {
        gpr: [0, 41, 0, 0, 0, 0, 0, 0x140002000, 0, 0, 0, 0, 0, 0, 0, 0],
        rip: 0x140001000,
        rflags: 2,
    };
    assert_eq!(state, initial);
    assert_eq!(image.data, [0xa5; 8]);
    let mut trace = Vec::new();
    image
        .run(&mut state, 16, |state| trace.push(state.clone()))
        .unwrap();
    let mut expected = initial;
    expected.rip = 0x140001005;
    expected.gpr[0] = 1;
    assert_eq!(trace[0], expected);
    expected.rip = 0x140001007;
    expected.gpr[0] = 42;
    assert_eq!(trace[1], expected);
    expected.rip = 0x14000100a;
    assert_eq!(trace[2], expected);
    assert_eq!(trace.len(), 3);
    assert_eq!(state, expected);
    assert_eq!(image.data, [42, 0, 0, 0, 0, 0, 0, 0]);
}

#[test]
fn rejects_malformed_images() {
    for len in 0..FIXTURE.len() {
        assert_eq!(load(&FIXTURE[..len]).unwrap_err(), LoadError::Bounds);
    }
    for (offset, patch, error) in [
        (0, 0, LoadError::Header),
        (0x3f, 0xff, LoadError::Header),
        (0x80, 0, LoadError::Header),
        (0x84, 0, LoadError::Header),
        (0x94, 0, LoadError::Header),
        (0x98, 0, LoadError::Header),
        (0xb1, 1, LoadError::Header),
        (0xb7, 0xff, LoadError::Header), // Unsupported high image base.
        (0xd1, 0x20, LoadError::Header), // SizeOfImage excludes .data.
        (0x104, 17, LoadError::Directory),
        (0x110, 1, LoadError::Directory), // Import directory.
        (0x130, 1, LoadError::Directory), // Base relocation directory.
        (0x190, 0, LoadError::Section),
        (0x190, 65, LoadError::Section),
        (0x1bd, 0x10, LoadError::Section), // Overlapping RVA.
        (0x1c5, 0x02, LoadError::Section), // Overlapping raw data.
        (0x1c5, 0x06, LoadError::Section), // Raw span starts past EOF.
        (0x1b9, 0x02, LoadError::Section), // Unsupported zero-filled extent.
        (0x1af, 0xe0, LoadError::Section), // Writable code.
        (0xa8, 1, LoadError::Entry),       // Interior of MOV.
        (0xa9, 0x20, LoadError::Entry),    // Data address.
    ] {
        let mut image = FIXTURE.to_vec();
        image[offset] = patch;
        assert_eq!(load(&image).unwrap_err(), error, "offset {offset:#x}");
    }
}

#[test]
fn unknown_service_leaves_trap_unretired() {
    let mut bytes = FIXTURE.to_vec();
    bytes[0x201] = 2;
    let mut image = load(&bytes).unwrap();
    let mut state = image.initial(41);
    let mut trace = Vec::new();
    assert_eq!(
        image.run(&mut state, 16, |state| trace.push(state.clone())),
        Err(Fault {
            kind: cpu::FaultKind::UnknownService,
            rip: 0x140001005,
        })
    );
    assert_eq!(trace, [state.clone()]);
    assert_eq!(state.gpr[0], 2);
    assert_eq!(state.gpr[1], 41);
    assert_eq!(image.data, [0xa5; 8]);
}

#[test]
fn denies_cross_boundary_and_code_writes() {
    for address in [0x140002004, 0x140001000, u64::MAX] {
        let mut image = load(FIXTURE).unwrap();
        let mut state = image.initial(41);
        state.gpr[7] = address;
        assert_eq!(
            image.run(&mut state, 16, |_| {}),
            Err(Fault {
                kind: cpu::FaultKind::OutputRange,
                rip: 0x140001007,
            })
        );
        assert_eq!(image.data, [0xa5; 8]);
    }
}

#[test]
fn denies_fetch_from_data_or_interior() {
    for address in [0x140002000, 0x140001001, 0x14000100a, 0x140000000] {
        let mut image = load(FIXTURE).unwrap();
        let mut state = image.initial(41);
        state.rip = address;
        assert_eq!(
            image.run(&mut state, 16, |_| panic!("unexpected retirement")),
            Err(Fault {
                kind: cpu::FaultKind::CodeRange,
                rip: address,
            })
        );
    }
}

#[test]
fn service_uses_initial_rcx_and_wraps() {
    for (argument, result, bytes) in [
        (0, 1, [1, 0, 0, 0, 0, 0, 0, 0]),
        (255, 256, [0, 1, 0, 0, 0, 0, 0, 0]),
        (u64::MAX, 0, [0; 8]),
    ] {
        let mut image = load(FIXTURE).unwrap();
        let mut state = image.initial(argument);
        state.gpr[0] = u64::MAX;
        let mut trace = Vec::new();
        image
            .run(&mut state, 16, |state| {
                trace.push((state.rip, state.gpr[0], state.gpr[1], state.rflags));
            })
            .unwrap();
        assert_eq!(
            trace,
            [
                (0x140001005, 1, argument, 2),
                (0x140001007, result, argument, 2),
                (0x14000100a, result, argument, 2),
            ]
        );
        assert_eq!(image.data, bytes);
    }
}

#[test]
fn ignores_raw_alignment_padding() {
    let mut bytes = FIXTURE.to_vec();
    bytes[0x20a..0x400].fill(0xcc);
    bytes[0x408..0x600].fill(0xff);
    let mut image = load(&bytes).unwrap();
    assert_eq!(image.data, [0xa5; 8]);
    let mut state = image.initial(41);
    let mut retired = 0;
    image.run(&mut state, 16, |_| retired += 1).unwrap();
    assert_eq!(retired, 3);
    assert_eq!(state.rip, 0x14000100a);
    assert_eq!(image.data, [42, 0, 0, 0, 0, 0, 0, 0]);
}

#[test]
fn truncated_mov_cannot_retire_or_invoke_service() {
    let mut bytes = FIXTURE.to_vec();
    bytes[0x190] = 4;
    let mut image = load(&bytes).unwrap();
    let mut state = image.initial(41);
    let initial = state.clone();
    assert_eq!(
        image.run(&mut state, 16, |_| panic!("unexpected retirement")),
        Err(Fault {
            kind: cpu::FaultKind::Truncated,
            rip: 0x140001000,
        })
    );
    assert_eq!(state, initial);
    assert_eq!(image.data, [0xa5; 8]);
}
