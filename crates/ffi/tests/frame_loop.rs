// SPDX-License-Identifier: GPL-3.0-or-later

//! Gate 3 acceptance on a native Metal device: persistent guest rendering through
//! the C interface. Needs `--features gpu-tests` (scripts/test-gpu.sh).
#![cfg(feature = "gpu-tests")]

use std::sync::Mutex;
use std::time::{Duration, Instant};
use xenora_ffi::*;

/// Tests share the process-wide session table and one GPU; run them one at a time.
static SERIAL: Mutex<()> = Mutex::new(());

/// Expected frame `id`, derived only from the geometry: the guest's frame index picks
/// column `id % 4`, which is opaque red (x9-constant-ps exports (1, 0, 0, 1)); the
/// other columns keep the clear colour (19, 43, 93, 251).
fn expected(id: u32) -> Vec<u8> {
    let mut frame = Vec::new();
    for _row in 0..3 {
        for column in 0..4 {
            if column == id % 4 {
                frame.extend([255, 0, 0, 255]);
            } else {
                frame.extend([19, 43, 93, 251]);
            }
        }
    }
    frame
}

struct Status {
    state: u32,
    completed: u32,
    frame_id: u32,
    rgba: [u8; 48],
}

fn status(session: u64) -> Status {
    let mut s = Status {
        state: 99,
        completed: 0,
        frame_id: 0,
        rgba: [0; 48],
    };
    let code = unsafe {
        xenora_session_frame_loop_status(
            session,
            &mut s.state,
            &mut s.completed,
            &mut s.frame_id,
            s.rgba.as_mut_ptr(),
            48,
        )
    };
    assert_eq!(code, XENORA_OK);
    s
}

fn session() -> u64 {
    let mut session = 0;
    assert_eq!(
        unsafe { xenora_session_create(XENORA_ABI_VERSION, &mut session) },
        XENORA_OK
    );
    session
}

fn wait_until(session: u64, what: &str, done: impl Fn(&Status) -> bool) -> Status {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let s = status(session);
        if done(&s) {
            return s;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for {what} (state {}, completed {})",
            s.state,
            s.completed
        );
        std::thread::sleep(Duration::from_millis(2));
    }
}

/// Physical memory footprint of this process (macOS `proc_pid_rusage`, RUSAGE_INFO_V2).
fn footprint() -> u64 {
    unsafe extern "C" {
        fn proc_pid_rusage(pid: i32, flavor: i32, buffer: *mut [u64; 32]) -> i32;
    }
    let mut info = [0u64; 32];
    assert_eq!(
        unsafe { proc_pid_rusage(std::process::id() as i32, 2, &mut info) },
        0
    );
    info[9] // after the 16-byte UUID: user, system, idle wakeups, interrupts, pageins, wired, resident, footprint
}

#[test]
fn three_hundred_guest_frames_arrive_verified_and_in_order() {
    let _serial = SERIAL.lock().unwrap_or_else(|p| p.into_inner());
    let session = session();
    assert_eq!(xenora_session_start_frame_loop(session, 300), XENORA_OK);
    assert_eq!(
        xenora_session_start_frame_loop(session, 300),
        XENORA_INVALID_STATE
    );
    let s = wait_until(session, "300 frames", |s| s.state != XENORA_LOOP_RUNNING);
    assert_eq!(s.state, XENORA_LOOP_FINISHED);
    assert_eq!(s.completed, 300);
    assert_eq!(s.frame_id, 299);
    assert_eq!(s.rgba.to_vec(), expected(299));
    assert_eq!(xenora_session_destroy(session), XENORA_OK);
}

#[test]
fn observed_frame_ids_never_go_backwards_and_match_their_pixels() {
    let _serial = SERIAL.lock().unwrap_or_else(|p| p.into_inner());
    let session = session();
    assert_eq!(xenora_session_start_frame_loop(session, 2_000), XENORA_OK);
    let mut previous = None;
    let mut samples = 0;
    loop {
        let s = status(session);
        if s.frame_id != XENORA_NO_FRAME {
            if let Some(p) = previous {
                assert!(s.frame_id >= p, "frame id went from {p} to {}", s.frame_id);
            }
            assert!(
                s.frame_id < s.completed,
                "id {} not below completed {}",
                s.frame_id,
                s.completed
            );
            assert_eq!(
                s.rgba.to_vec(),
                expected(s.frame_id),
                "frame {}",
                s.frame_id
            );
            previous = Some(s.frame_id);
            samples += 1;
        }
        if s.state != XENORA_LOOP_RUNNING {
            assert_eq!(s.state, XENORA_LOOP_FINISHED);
            assert_eq!((s.completed, s.frame_id), (2_000, 1_999));
            break;
        }
    }
    assert!(samples > 1);
    xenora_session_destroy(session);
}

#[test]
fn pause_holds_the_guest_resume_continues_and_stop_halts_it() {
    let _serial = SERIAL.lock().unwrap_or_else(|p| p.into_inner());
    let session = session();
    assert_eq!(
        xenora_session_start_frame_loop(session, XENORA_FRAME_LOOP_MAX_FRAMES),
        XENORA_OK
    );
    wait_until(session, "10 frames", |s| s.completed >= 10);

    assert_eq!(xenora_session_pause_frame_loop(session), XENORA_OK);
    // At most the frame already being rendered can still complete after pausing.
    std::thread::sleep(Duration::from_millis(50));
    let held = status(session);
    std::thread::sleep(Duration::from_millis(200));
    let later = status(session);
    assert_eq!(later.state, XENORA_LOOP_PAUSED);
    assert_eq!(later.completed, held.completed, "no frames while paused");
    assert_eq!(later.frame_id, held.completed - 1);
    assert_eq!(later.rgba.to_vec(), expected(later.frame_id));

    assert_eq!(xenora_session_resume_frame_loop(session), XENORA_OK);
    let resumed = wait_until(session, "progress after resume", |s| {
        s.completed >= held.completed + 10
    });
    assert_eq!(resumed.state, XENORA_LOOP_RUNNING);

    assert_eq!(xenora_session_stop_frame_loop(session), XENORA_OK);
    let stopped = status(session);
    assert_eq!(stopped.state, XENORA_LOOP_STOPPED);
    assert!(stopped.completed < XENORA_FRAME_LOOP_MAX_FRAMES);
    assert_eq!(stopped.frame_id, stopped.completed - 1);
    assert_eq!(stopped.rgba.to_vec(), expected(stopped.frame_id));
    // Pause and resume are refused once the guest has halted.
    assert_eq!(
        xenora_session_pause_frame_loop(session),
        XENORA_INVALID_STATE
    );
    assert_eq!(
        xenora_session_resume_frame_loop(session),
        XENORA_INVALID_STATE
    );
    assert_eq!(xenora_session_destroy(session), XENORA_OK);
}

#[test]
fn destroying_a_running_or_paused_session_stops_it() {
    let _serial = SERIAL.lock().unwrap_or_else(|p| p.into_inner());
    for pause_first in [false, true] {
        let session = session();
        assert_eq!(
            xenora_session_start_frame_loop(session, XENORA_FRAME_LOOP_MAX_FRAMES),
            XENORA_OK
        );
        wait_until(session, "a frame", |s| s.completed >= 1);
        if pause_first {
            assert_eq!(xenora_session_pause_frame_loop(session), XENORA_OK);
        }
        let started = Instant::now();
        assert_eq!(xenora_session_destroy(session), XENORA_OK);
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "destroy joined promptly"
        );
        let mut x = 0;
        let mut rgba = [0u8; 48];
        assert_eq!(
            unsafe {
                xenora_session_frame_loop_status(
                    session,
                    &mut x,
                    &mut x,
                    &mut x,
                    rgba.as_mut_ptr(),
                    48,
                )
            },
            XENORA_INVALID_HANDLE
        );
    }
}

#[test]
fn repeated_sessions_do_not_grow_the_memory_footprint() {
    let _serial = SERIAL.lock().unwrap_or_else(|p| p.into_inner());
    let run = || {
        let session = session();
        assert_eq!(xenora_session_start_frame_loop(session, 300), XENORA_OK);
        let s = wait_until(session, "300 frames", |s| s.state != XENORA_LOOP_RUNNING);
        assert_eq!((s.state, s.completed), (XENORA_LOOP_FINISHED, 300));
        assert_eq!(xenora_session_destroy(session), XENORA_OK);
    };
    // Warm up so one-time allocations (Metal device, shader compiler) are excluded.
    run();
    run();
    let baseline = footprint();
    for _ in 0..5 {
        run();
    }
    let after = footprint();
    let growth = after.saturating_sub(baseline);
    println!("footprint baseline={baseline} after_5_sessions={after} growth={growth}");
    assert!(
        growth < 16 << 20,
        "footprint grew by {growth} bytes over 5 sessions of 300 frames"
    );
}
