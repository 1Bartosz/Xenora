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

// ---------------------------------------------------------------------------
// New tests: ServiceRegistry — typed multi-service dispatch
// ---------------------------------------------------------------------------

/// Helper: build the standard FIXTURE image, run it with a registry, return
/// (trace of (rip,rax,rcx), final data, result).
#[allow(clippy::type_complexity)]
fn run_with_registry_fixture(
    argument: u64,
    registry: &mut ServiceRegistry,
) -> (Vec<(u64, u64, u64)>, [u8; 8], Result<(), Fault>) {
    let mut image = load(FIXTURE).unwrap();
    let mut state = image.initial(argument);
    let mut trace = Vec::new();
    let result = image.run_with_registry(&mut state, 16, registry, |s| {
        trace.push((s.rip, s.gpr[0], s.gpr[1]));
    });
    (trace, image.data, result)
}

/// Service ID 1 dispatched through registry returns RCX+1, same as the
/// hardcoded `run` path.  This confirms the registry correctly routes by EAX.
#[test]
fn registry_service_one_matches_hardcoded_run() {
    let mut registry = ServiceRegistry::new();
    registry.register(
        1,
        ServiceEntry {
            name: "increment",
            handler: Box::new(|state| Ok(state.gpr[1].wrapping_add(1))),
        },
    );
    let (trace, data, result) = run_with_registry_fixture(41, &mut registry);
    result.unwrap();
    // MOV EAX,1 → UD2(service 1, rcx=41→42) → STORE → three retirements.
    assert_eq!(
        trace,
        [
            (0x140001005, 1, 41),
            (0x140001007, 42, 41),
            (0x14000100a, 42, 41),
        ]
    );
    assert_eq!(data, [42, 0, 0, 0, 0, 0, 0, 0]);
}

/// A second service (ID 2, doubles RCX) is registered alongside service 1.
/// A patched fixture uses service 2: EAX=2 at UD2 must route to that handler.
/// Independently derived: service 2(rcx=10) → 20 stored in .data.
///
/// Patch: change the `MOV EAX,1` immediate in .text from 1 to 2 so the guest
/// uses service 2 instead of service 1.
#[test]
fn registry_dispatches_second_service_by_eax() {
    // The x2 fixture .text starts at file offset 0x200.
    // Bytes: B8 01 00 00 00 (MOV EAX,1) — patch immediate to 2.
    let mut bytes = FIXTURE.to_vec();
    bytes[0x201] = 2; // change service ID from 1 to 2

    let mut image = load(&bytes).unwrap();
    let mut state = image.initial(10); // RCX=10
    let mut registry = ServiceRegistry::new();
    registry.register(
        1,
        ServiceEntry {
            name: "increment",
            handler: Box::new(|state| Ok(state.gpr[1].wrapping_add(1))),
        },
    );
    registry.register(
        2,
        ServiceEntry {
            name: "double",
            handler: Box::new(|state| Ok(state.gpr[1].wrapping_mul(2))),
        },
    );
    let mut trace = Vec::new();
    let result = image.run_with_registry(&mut state, 16, &mut registry, |s| {
        trace.push((s.rip, s.gpr[0], s.gpr[1]));
    });
    result.unwrap();
    // MOV EAX,2 → UD2(service 2, rcx=10→20) → STORE(20) → 3 retirements.
    assert_eq!(
        trace,
        [
            (0x140001005, 2, 10),
            (0x140001007, 20, 10),
            (0x14000100a, 20, 10),
        ]
    );
    assert_eq!(image.data, [20, 0, 0, 0, 0, 0, 0, 0]);
}

/// An unknown service ID (not registered) produces `UnknownService` before the
/// trap retires; .data remains at the initial sentinel value.
/// Independently: EAX=99 at UD2 → no matching entry → fault at trap RIP.
#[test]
fn registry_unknown_id_faults_before_retire() {
    let mut registry = ServiceRegistry::new();
    registry.register(
        1,
        ServiceEntry {
            name: "increment",
            handler: Box::new(|state| Ok(state.gpr[1].wrapping_add(1))),
        },
    );
    // Patch service ID to 99 (not registered).
    let mut bytes = FIXTURE.to_vec();
    bytes[0x201] = 99;
    let (trace, data, result) = {
        let mut image = load(&bytes).unwrap();
        let mut state = image.initial(5);
        let mut trace = Vec::new();
        let result = image.run_with_registry(&mut state, 16, &mut registry, |s| {
            trace.push((s.rip, s.gpr[0]));
        });
        (trace, image.data, result)
    };
    // Only MOV EAX,99 retires; UD2 faults before retirement.
    assert_eq!(trace, [(0x140001005, 99,)]);
    assert_eq!(
        result,
        Err(Fault {
            kind: cpu::FaultKind::UnknownService,
            rip: 0x140001005,
        })
    );
    assert_eq!(data, [0xa5; 8]); // initial sentinel preserved
}

/// A handler that returns an error for an out-of-range argument propagates
/// that error as a fault; the trap and store do not retire.
/// Independently: service 1 rejects RCX=u64::MAX → fault at trap RIP.
#[test]
fn registry_handler_argument_rejection() {
    let mut registry = ServiceRegistry::new();
    registry.register(
        1,
        ServiceEntry {
            name: "bounded_increment",
            // Reject if RCX would overflow u32 on increment.
            handler: Box::new(|state| {
                let arg = state.gpr[1];
                if arg > u64::from(u32::MAX) {
                    return Err(cpu::FaultKind::UnknownService);
                }
                Ok(arg.wrapping_add(1))
            }),
        },
    );
    let (trace, data, result) = run_with_registry_fixture(u64::MAX, &mut registry);
    // MOV EAX,1 retires; UD2 handler rejects argument; no store.
    assert_eq!(trace, [(0x140001005, 1, u64::MAX)]);
    assert_eq!(
        result,
        Err(Fault {
            kind: cpu::FaultKind::UnknownService,
            rip: 0x140001005,
        })
    );
    assert_eq!(data, [0xa5; 8]);
}

/// Registering a duplicate service ID must panic (programmer error).
#[test]
#[should_panic(expected = "duplicate service ID 1")]
fn registry_duplicate_id_panics() {
    let mut registry = ServiceRegistry::new();
    registry.register(
        1,
        ServiceEntry {
            name: "first",
            handler: Box::new(|_| Ok(0)),
        },
    );
    registry.register(
        1,
        ServiceEntry {
            name: "duplicate",
            handler: Box::new(|_| Ok(0)),
        },
    );
}

/// Registering service ID 0 must panic (reserved).
#[test]
#[should_panic(expected = "service ID 0 is reserved")]
fn registry_zero_id_panics() {
    let mut registry = ServiceRegistry::new();
    registry.register(
        0,
        ServiceEntry {
            name: "zero",
            handler: Box::new(|_| Ok(0)),
        },
    );
}
