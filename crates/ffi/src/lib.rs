// SPDX-License-Identifier: GPL-3.0-or-later

//! Versioned C interface for the native macOS application (see `include/xenora.h`).
//!
//! Sessions are opaque 64-bit handles: the low 32 bits index a slot and the high
//! 32 bits hold that slot's generation, so a stale handle never reaches a reused slot.
//! Every exported function catches panics and reports them as `XENORA_INTERNAL_ERROR`.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Arc, Mutex};
use xenora_loader::startup::{StartupLayout, run_synthetic_startup};
use xenora_loader::{ExitReason, ImportRegistry, ParsedPe, Register};
use xenora_metal::NativeCompute;
use xenora_shader::{Architecture, PIXEL_ENTRY_POINT, VERTEX_ENTRY_POINT, compile_graphics};

pub const XENORA_ABI_VERSION: u32 = 1;

pub const XENORA_OK: i32 = 0;
pub const XENORA_INVALID_ARGUMENT: i32 = 1;
pub const XENORA_INVALID_HANDLE: i32 = 2;
pub const XENORA_VERSION_MISMATCH: i32 = 3;
pub const XENORA_INTERNAL_ERROR: i32 = 4;
pub const XENORA_PE_ERROR: i32 = 5;
pub const XENORA_LIMIT_EXCEEDED: i32 = 6;
pub const XENORA_BUFFER_TOO_SMALL: i32 = 7;
pub const XENORA_EXECUTION_FAILED: i32 = 8;

pub const XENORA_FIXTURE_FRAME_WIDTH: u32 = 4;
pub const XENORA_FIXTURE_FRAME_HEIGHT: u32 = 3;
const FRAME_BYTES: usize = (XENORA_FIXTURE_FRAME_WIDTH * XENORA_FIXTURE_FRAME_HEIGHT * 4) as usize;

/// Upper bound on simultaneously live sessions.
const MAX_SESSIONS: usize = 64;

const FIXTURE: &[u8] = include_bytes!("../../../fixtures/pe/trusted-runner.exe");

struct Slot {
    /// Odd while live, even while free; starts at 0 so the first handle is never 0.
    generation: u32,
}

static SESSIONS: Mutex<Vec<Slot>> = Mutex::new(Vec::new());

fn guarded(f: impl FnOnce() -> i32) -> i32 {
    catch_unwind(AssertUnwindSafe(f)).unwrap_or(XENORA_INTERNAL_ERROR)
}

fn split(handle: u64) -> (usize, u32) {
    ((handle & 0xFFFF_FFFF) as usize, (handle >> 32) as u32)
}

fn is_live(slots: &[Slot], handle: u64) -> bool {
    let (index, generation) = split(handle);
    slots
        .get(index)
        .is_some_and(|slot| slot.generation == generation && generation % 2 == 1)
}

#[unsafe(no_mangle)]
pub extern "C" fn xenora_abi_version() -> u32 {
    XENORA_ABI_VERSION
}

/// # Safety
/// `out_session` must be null or valid for one aligned `u64` write.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn xenora_session_create(abi_version: u32, out_session: *mut u64) -> i32 {
    guarded(|| {
        if out_session.is_null() {
            return XENORA_INVALID_ARGUMENT;
        }
        if abi_version != XENORA_ABI_VERSION {
            return XENORA_VERSION_MISMATCH;
        }
        let Ok(mut slots) = SESSIONS.lock() else {
            return XENORA_INTERNAL_ERROR;
        };
        let index = match slots.iter().position(|slot| slot.generation % 2 == 0) {
            Some(index) => index,
            None if slots.len() < MAX_SESSIONS => {
                slots.push(Slot { generation: 0 });
                slots.len() - 1
            }
            None => return XENORA_LIMIT_EXCEEDED,
        };
        let slot = &mut slots[index];
        // ponytail: a slot retires after 2^31 reuses instead of wrapping its generation.
        let Some(generation) = slot.generation.checked_add(1) else {
            return XENORA_LIMIT_EXCEEDED;
        };
        slot.generation = generation;
        // SAFETY: checked non-null above; the caller guarantees validity.
        unsafe { out_session.write((u64::from(generation) << 32) | index as u64) };
        XENORA_OK
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn xenora_session_destroy(session: u64) -> i32 {
    guarded(|| {
        let Ok(mut slots) = SESSIONS.lock() else {
            return XENORA_INTERNAL_ERROR;
        };
        if !is_live(&slots, session) {
            return XENORA_INVALID_HANDLE;
        }
        slots[split(session).0].generation += 1;
        XENORA_OK
    })
}

/// # Safety
/// Each output pointer must be null or valid for one aligned `u32` write.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn xenora_session_inspect_trusted_fixture(
    session: u64,
    out_section_count: *mut u32,
    out_entry_rva: *mut u32,
) -> i32 {
    guarded(|| {
        if out_section_count.is_null() || out_entry_rva.is_null() {
            return XENORA_INVALID_ARGUMENT;
        }
        match SESSIONS.lock() {
            Ok(slots) if is_live(&slots, session) => {}
            Ok(_) => return XENORA_INVALID_HANDLE,
            Err(_) => return XENORA_INTERNAL_ERROR,
        }
        let Ok(pe) = ParsedPe::parse(FIXTURE) else {
            return XENORA_PE_ERROR;
        };
        // SAFETY: both checked non-null above; the caller guarantees validity.
        unsafe {
            out_section_count.write(u32::from(pe.number_of_sections));
            out_entry_rva.write(pe.entry_point_rva);
        }
        XENORA_OK
    })
}

// The trusted fixture's contract, as checked by the CLI runner (crates/cli/src/runner.rs).
// ponytail: duplicated from the runner so the CLI stays untouched; extract a shared
// fixture-run function into a library when a third caller appears.
const IMAGE_BASE: u64 = 0x1_5000_0000;
const TICKET_ADDRESS: u64 = IMAGE_BASE + 0x3000;
const TICKET: u64 = 17;
const DRAW_RESULT: u64 = 0x42;
const ENTRY_STEPS: usize = 100;
/// `x9-constant-ps.s` exports (1.0, 0, 0, 1.0) to mrt0; in an RGBA8 target that is opaque red.
const EXPECTED_PIXEL: [u8; 4] = [255, 0, 0, 255];

fn draw() -> Result<Vec<u8>, String> {
    let shader = compile_graphics(
        Architecture::Gfx1030,
        include_bytes!("../../../fixtures/rdna2/x9-constant-vs.bin"),
        include_bytes!("../../../fixtures/rdna2/x9-constant-ps.bin"),
    )
    .map_err(|e| format!("shader translation: {e}"))?;
    let gpu = NativeCompute::new().map_err(|e| format!("Metal device: {e}"))?;
    let pixels = gpu
        .render_rgba8(
            &shader.msl,
            [VERTEX_ENTRY_POINT, PIXEL_ENTRY_POINT],
            &[
                [-1.0, -1.0, 0.5, 1.0],
                [3.0, -1.0, 0.5, 1.0],
                [-1.0, 3.0, 0.5, 1.0],
            ],
            &[[0.0; 4]; 3],
            [
                XENORA_FIXTURE_FRAME_WIDTH as usize,
                XENORA_FIXTURE_FRAME_HEIGHT as usize,
            ],
            [19, 43, 93, 251],
        )
        .map_err(|e| format!("Metal render: {e}"))?;
    if pixels != EXPECTED_PIXEL.repeat(FRAME_BYTES / 4) {
        return Err("read-back frame differs from the fixture shader's output".into());
    }
    Ok(pixels)
}

/// Run the embedded fixture and return its verified frame.
fn run_fixture() -> Result<Vec<u8>, String> {
    let frame = Arc::new(Mutex::new(None));
    let draw_frame = Arc::clone(&frame);
    let mut imports = ImportRegistry::new();
    imports.register("xenora.dll", "fixture_draw", move |args, _, memory| {
        let fault = ExitReason::IllegalInstruction(0xff);
        let ticket = args.get(0, memory).map_err(ExitReason::MemoryFault)?;
        let mut slot = draw_frame.lock().map_err(|_| fault)?;
        if slot.is_some()
            || ticket != TICKET_ADDRESS
            || memory.read_u64(ticket).map_err(ExitReason::MemoryFault)? != TICKET
        {
            return Err(fault);
        }
        let result = draw();
        let ok = result.is_ok();
        *slot = Some(result);
        if ok { Ok(DRAW_RESULT) } else { Err(fault) }
    });
    let startup = run_synthetic_startup(
        FIXTURE,
        StartupLayout {
            image_base: IMAGE_BASE,
            stack_top: 0x2_0001_0000,
            tls_base: 0x3_0000_0000,
        },
        imports,
        ENTRY_STEPS,
    )
    .map_err(|e| format!("startup: {e}"))?;
    let drawn = frame.lock().map_err(|_| "frame lock poisoned")?.take();
    if let Some(Err(e)) = drawn {
        return Err(e);
    }
    let execution = &startup.execution;
    if execution.exit_reason != ExitReason::Halt
        || startup.relocations.applied_dir64 != 4
        || startup.bound_imports != 1
        || startup.tls_callbacks != 1
        || execution.final_state.get(Register::Rax) != DRAW_RESULT
    {
        return Err(format!(
            "unexpected guest result: {:?}",
            execution.exit_reason
        ));
    }
    drawn.ok_or_else(|| String::from("guest halted without a draw"))?
}

/// # Safety
/// `out_rgba` must be null or valid for `capacity` byte writes; each `u32` output
/// must be null or valid for one aligned write.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn xenora_session_run_trusted_fixture(
    session: u64,
    out_rgba: *mut u8,
    capacity: u64,
    out_width: *mut u32,
    out_height: *mut u32,
) -> i32 {
    guarded(|| {
        if out_rgba.is_null() || out_width.is_null() || out_height.is_null() {
            return XENORA_INVALID_ARGUMENT;
        }
        match SESSIONS.lock() {
            Ok(slots) if is_live(&slots, session) => {}
            Ok(_) => return XENORA_INVALID_HANDLE,
            Err(_) => return XENORA_INTERNAL_ERROR,
        }
        if capacity < FRAME_BYTES as u64 {
            return XENORA_BUFFER_TOO_SMALL;
        }
        let Ok(pixels) = run_fixture() else {
            return XENORA_EXECUTION_FAILED;
        };
        // SAFETY: non-null and at least FRAME_BYTES long, both checked above.
        unsafe {
            std::ptr::copy_nonoverlapping(pixels.as_ptr(), out_rgba, FRAME_BYTES);
            out_width.write(XENORA_FIXTURE_FRAME_WIDTH);
            out_height.write(XENORA_FIXTURE_FRAME_HEIGHT);
        }
        XENORA_OK
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ptr::null_mut;

    /// The session table is process-global, so tests that fill or count it run one at a time.
    static SERIAL: Mutex<()> = Mutex::new(());

    fn serial() -> std::sync::MutexGuard<'static, ()> {
        SERIAL
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn create() -> u64 {
        let mut handle = 0;
        assert_eq!(
            unsafe { xenora_session_create(XENORA_ABI_VERSION, &mut handle) },
            XENORA_OK
        );
        assert_ne!(handle, 0);
        handle
    }

    fn inspect(handle: u64) -> (i32, u32, u32) {
        let (mut sections, mut entry) = (u32::MAX, u32::MAX);
        let status =
            unsafe { xenora_session_inspect_trusted_fixture(handle, &mut sections, &mut entry) };
        (status, sections, entry)
    }

    #[test]
    fn header_and_rust_constants_agree() {
        let header = include_str!("../include/xenora.h");
        assert!(header.contains(&format!("#define XENORA_ABI_VERSION {XENORA_ABI_VERSION}u")));
        for (name, value) in [
            ("XENORA_OK", XENORA_OK),
            ("XENORA_INVALID_ARGUMENT", XENORA_INVALID_ARGUMENT),
            ("XENORA_INVALID_HANDLE", XENORA_INVALID_HANDLE),
            ("XENORA_VERSION_MISMATCH", XENORA_VERSION_MISMATCH),
            ("XENORA_INTERNAL_ERROR", XENORA_INTERNAL_ERROR),
            ("XENORA_PE_ERROR", XENORA_PE_ERROR),
            ("XENORA_LIMIT_EXCEEDED", XENORA_LIMIT_EXCEEDED),
            ("XENORA_BUFFER_TOO_SMALL", XENORA_BUFFER_TOO_SMALL),
            ("XENORA_EXECUTION_FAILED", XENORA_EXECUTION_FAILED),
        ] {
            assert!(header.contains(&format!("{name} = {value},")), "{name}");
        }
        assert!(header.contains(&format!(
            "XENORA_FIXTURE_FRAME_WIDTH {XENORA_FIXTURE_FRAME_WIDTH}u"
        )));
        assert!(header.contains(&format!(
            "XENORA_FIXTURE_FRAME_HEIGHT {XENORA_FIXTURE_FRAME_HEIGHT}u"
        )));
    }

    #[test]
    fn session_inspects_the_trusted_fixture() {
        let _serial = serial();
        let handle = create();
        // Independent expectation: read straight from the fixture's COFF and optional headers.
        let lfanew = u32::from_le_bytes(FIXTURE[0x3C..0x40].try_into().unwrap()) as usize;
        let sections = u16::from_le_bytes(FIXTURE[lfanew + 6..lfanew + 8].try_into().unwrap());
        let entry = u32::from_le_bytes(FIXTURE[lfanew + 40..lfanew + 44].try_into().unwrap());
        assert_eq!(inspect(handle), (XENORA_OK, u32::from(sections), entry));
        assert_eq!(xenora_session_destroy(handle), XENORA_OK);
    }

    #[test]
    fn version_mismatch_and_null_outputs_are_rejected() {
        let _serial = serial();
        let mut handle = 7;
        assert_eq!(
            unsafe { xenora_session_create(2, &mut handle) },
            XENORA_VERSION_MISMATCH
        );
        assert_eq!(handle, 7, "output untouched on failure");
        assert_eq!(
            unsafe { xenora_session_create(1, null_mut()) },
            XENORA_INVALID_ARGUMENT
        );
        let live = create();
        let mut x = 0;
        assert_eq!(
            unsafe { xenora_session_inspect_trusted_fixture(live, null_mut(), &mut x) },
            XENORA_INVALID_ARGUMENT
        );
        xenora_session_destroy(live);
    }

    #[test]
    fn invalid_stale_and_double_destroyed_handles_are_rejected() {
        let _serial = serial();
        for bogus in [0, u64::MAX, 0xFFFF_FFFF, 1 << 32] {
            assert_eq!(
                xenora_session_destroy(bogus),
                XENORA_INVALID_HANDLE,
                "{bogus:#x}"
            );
            assert_eq!(inspect(bogus), (XENORA_INVALID_HANDLE, u32::MAX, u32::MAX));
        }
        let handle = create();
        assert_eq!(xenora_session_destroy(handle), XENORA_OK);
        assert_eq!(xenora_session_destroy(handle), XENORA_INVALID_HANDLE);
        assert_eq!(inspect(handle).0, XENORA_INVALID_HANDLE);
        // Reusing the slot must not revive the old handle.
        let reused: Vec<u64> = (0..MAX_SESSIONS).map(|_| create()).collect();
        assert!(!reused.contains(&handle));
        assert_eq!(inspect(handle).0, XENORA_INVALID_HANDLE);
        for h in reused {
            xenora_session_destroy(h);
        }
    }

    #[test]
    fn repeated_and_concurrent_lifecycles_do_not_leak_slots() {
        let _serial = serial();
        let threads: Vec<_> = (0..8)
            .map(|_| {
                std::thread::spawn(|| {
                    for _ in 0..500 {
                        let mut handle = 0;
                        let status = unsafe { xenora_session_create(1, &mut handle) };
                        assert_eq!(status, XENORA_OK);
                        assert_eq!(inspect(handle).0, XENORA_OK);
                        assert_eq!(xenora_session_destroy(handle), XENORA_OK);
                    }
                })
            })
            .collect();
        for t in threads {
            t.join().unwrap();
        }
        assert!(SESSIONS.lock().unwrap().len() <= MAX_SESSIONS);
    }

    #[test]
    fn panics_do_not_cross_the_boundary() {
        assert_eq!(guarded(|| panic!("injected")), XENORA_INTERNAL_ERROR);
    }

    #[test]
    fn fixture_run_rejects_bad_arguments_before_executing() {
        let _serial = serial();
        let handle = create();
        let (mut buf, mut w, mut h) = ([0u8; 48], 0u32, 0u32);
        let run = |session, buf: *mut u8, cap, w: *mut u32, h: *mut u32| unsafe {
            xenora_session_run_trusted_fixture(session, buf, cap, w, h)
        };
        assert_eq!(
            run(handle, buf.as_mut_ptr(), 47, &mut w, &mut h),
            XENORA_BUFFER_TOO_SMALL
        );
        assert_eq!(
            run(handle, null_mut(), 48, &mut w, &mut h),
            XENORA_INVALID_ARGUMENT
        );
        assert_eq!(
            run(handle, buf.as_mut_ptr(), 48, null_mut(), &mut h),
            XENORA_INVALID_ARGUMENT
        );
        xenora_session_destroy(handle);
        assert_eq!(
            run(handle, buf.as_mut_ptr(), 48, &mut w, &mut h),
            XENORA_INVALID_HANDLE
        );
        assert_eq!((w, h), (0, 0), "outputs untouched on failure");
        assert!(buf.iter().all(|&b| b == 0));
    }
}
