// SPDX-License-Identifier: GPL-3.0-or-later

//! Versioned C interface for the native macOS application (see `include/xenora.h`).
//!
//! Sessions are opaque 64-bit handles: the low 32 bits index a slot and the high
//! 32 bits hold that slot's generation, so a stale handle never reaches a reused slot.
//! Every exported function catches panics and reports them as `XENORA_INTERNAL_ERROR`.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Mutex;
use xenora_loader::ParsedPe;

pub const XENORA_ABI_VERSION: u32 = 1;

pub const XENORA_OK: i32 = 0;
pub const XENORA_INVALID_ARGUMENT: i32 = 1;
pub const XENORA_INVALID_HANDLE: i32 = 2;
pub const XENORA_VERSION_MISMATCH: i32 = 3;
pub const XENORA_INTERNAL_ERROR: i32 = 4;
pub const XENORA_PE_ERROR: i32 = 5;
pub const XENORA_LIMIT_EXCEEDED: i32 = 6;

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
        ] {
            assert!(header.contains(&format!("{name} = {value},")), "{name}");
        }
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
}
