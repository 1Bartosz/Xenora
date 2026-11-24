// SPDX-License-Identifier: GPL-3.0-or-later

//! Gate 4 acceptance on a native Metal device: interactive guest execution with
//! dynamic input handling and rendering through the C interface.
//! Needs `--features gpu-tests` (scripts/test-gpu.sh).
#![cfg(feature = "gpu-tests")]

use std::sync::Mutex;
use std::time::{Duration, Instant};
use xenora_ffi::*;

/// Tests share the process-wide session table and one GPU; run them one at a time.
static SERIAL: Mutex<()> = Mutex::new(());

/// Expected frame `column` (0..3): column `column` is opaque red (255, 0, 0, 255);
/// all other columns keep the clear colour (19, 43, 93, 251).
fn expected(column: u32) -> Vec<u8> {
    let mut frame = Vec::new();
    for _row in 0..3 {
        for col in 0..4 {
            if col == column % 4 {
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

fn send_buttons(session: u64, buttons: u32) {
    let input = xenora_input_state {
        buttons,
        stick_x: 0,
        stick_y: 0,
        mouse_dx: 0,
        mouse_dy: 0,
        mouse_buttons: 0,
        connected: 1,
        reserved: 0,
    };
    assert_eq!(
        unsafe { xenora_session_send_input(session, &input) },
        XENORA_OK
    );
}

fn queue_buttons(session: u64, buttons: u32) {
    let input = xenora_input_state {
        buttons,
        stick_x: 0,
        stick_y: 0,
        mouse_dx: 0,
        mouse_dy: 0,
        mouse_buttons: 0,
        connected: 1,
        reserved: 0,
    };
    assert_eq!(
        unsafe { xenora_session_queue_input(session, &input) },
        XENORA_OK
    );
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

#[test]
fn interactive_input_moves_guest_column_and_updates_pixels() {
    let _serial = SERIAL.lock().unwrap_or_else(|p| p.into_inner());
    let session = session();

    // Start interactive loop with 120 frames
    assert_eq!(
        xenora_session_start_interactive_loop(session, 120),
        XENORA_OK
    );

    // Initial frames without input stay at column 0
    let s = wait_until(session, "initial frames", |s| s.completed >= 5);
    assert_eq!(s.frame_id, 0);
    assert_eq!(s.rgba.to_vec(), expected(0));

    // Send Right input: column should advance 0 -> 1 -> 2 -> 3 -> 0
    send_buttons(session, XENORA_INPUT_RIGHT);
    let s_right = wait_until(session, "column moves right", |s| {
        s.completed >= 15 && s.frame_id != 0
    });
    assert_eq!(s_right.rgba.to_vec(), expected(s_right.frame_id));

    // Release input: position stops moving
    send_buttons(session, XENORA_INPUT_NONE);
    let s_hold_start = wait_until(session, "wait for release frame", |s| {
        s.completed >= s_right.completed + 2
    });
    let hold_col = s_hold_start.frame_id;
    let s_hold = wait_until(session, "position holds on release", |s| {
        s.completed >= s_hold_start.completed + 10
    });
    assert_eq!(s_hold.frame_id, hold_col);
    assert_eq!(s_hold.rgba.to_vec(), expected(s_hold.frame_id));

    // Send Left input: column should move left
    send_buttons(session, XENORA_INPUT_LEFT);
    let s_left = wait_until(session, "column moves left", |s| {
        s.completed >= s_hold.completed + 10 && s.frame_id != hold_col
    });
    assert_eq!(s_left.rgba.to_vec(), expected(s_left.frame_id));

    // Wait until finished
    let s_end = wait_until(session, "loop finish", |s| s.state == XENORA_LOOP_FINISHED);
    assert_eq!(s_end.completed, 120);

    assert_eq!(xenora_session_destroy(session), XENORA_OK);
}

#[test]
fn deterministic_input_replay_produces_identical_frames() {
    let _serial = SERIAL.lock().unwrap_or_else(|p| p.into_inner());

    // Timeline of inputs across 60 frames:
    // frames 0..15: Right
    // frames 15..30: None
    // frames 30..45: Left
    // frames 45..60: Right
    let replay_sequence: Vec<u32> = (0..60)
        .map(|f| {
            if f < 15 {
                XENORA_INPUT_RIGHT
            } else if f < 30 {
                XENORA_INPUT_NONE
            } else if f < 45 {
                XENORA_INPUT_LEFT
            } else {
                XENORA_INPUT_RIGHT
            }
        })
        .collect();

    let run_replay = || {
        let session = session();
        assert_eq!(
            xenora_session_start_interactive_loop(session, 60),
            XENORA_OK
        );

        // Pre-queue the complete input sequence
        for &btn in &replay_sequence {
            queue_buttons(session, btn);
        }

        // Wait until all 60 frames are rendered
        let s = wait_until(session, "replay completion", |s| {
            s.state == XENORA_LOOP_FINISHED && s.completed == 60
        });
        assert_eq!(s.completed, 60);

        // Inspect all 60 recorded frames
        let mut frames = Vec::with_capacity(60);
        for i in 0..60 {
            let mut id = 0u32;
            let mut rgba = [0u8; 48];
            let code = unsafe {
                xenora_session_get_recorded_frame(session, i, &mut id, rgba.as_mut_ptr(), 48)
            };
            assert_eq!(code, XENORA_OK);
            assert_eq!(rgba.to_vec(), expected(id));
            frames.push((id, rgba));
        }

        assert_eq!(xenora_session_destroy(session), XENORA_OK);
        frames
    };

    let run1 = run_replay();
    let run2 = run_replay();

    assert_eq!(run1.len(), 60);
    assert_eq!(run2.len(), 60);
    for i in 0..60 {
        assert_eq!(
            run1[i].0, run2[i].0,
            "frame {} column mismatch: {} vs {}",
            i, run1[i].0, run2[i].0
        );
        assert_eq!(run1[i].1, run2[i].1, "frame {} pixel mismatch", i);
    }
}

#[test]
fn focus_loss_and_release_resets_input_cleanly() {
    let _serial = SERIAL.lock().unwrap_or_else(|p| p.into_inner());
    let session = session();

    assert_eq!(
        xenora_session_start_interactive_loop(session, 100),
        XENORA_OK
    );

    // Send Right
    send_buttons(session, XENORA_INPUT_RIGHT);
    let s1 = wait_until(session, "moving right", |s| s.completed >= 10);

    // Simulate focus loss / key release
    send_buttons(session, XENORA_INPUT_NONE);
    // Wait for the release to take effect in the guest
    let s_released = wait_until(session, "release takes effect", |s| {
        s.completed >= s1.completed + 2
    });
    let stopped_col = s_released.frame_id;

    // Check next 15 frames: frame_id must remain fixed at stopped_col
    let s2 = wait_until(session, "hold after release", |s| {
        s.completed >= s_released.completed + 15
    });
    assert_eq!(s2.frame_id, stopped_col);
    assert_eq!(s2.rgba.to_vec(), expected(stopped_col));

    // Clean stop
    assert_eq!(xenora_session_stop_frame_loop(session), XENORA_OK);
    let s_stopped = wait_until(session, "stopped", |s| s.state == XENORA_LOOP_STOPPED);
    assert_eq!(s_stopped.state, XENORA_LOOP_STOPPED);

    assert_eq!(xenora_session_destroy(session), XENORA_OK);
}

#[test]
fn destroying_interactive_session_stops_cleanly() {
    let _serial = SERIAL.lock().unwrap_or_else(|p| p.into_inner());
    let session = session();

    assert_eq!(
        xenora_session_start_interactive_loop(session, 5000),
        XENORA_OK
    );

    // Wait until it actually starts rendering
    wait_until(session, "running", |s| s.completed >= 5);

    let start = Instant::now();
    assert_eq!(xenora_session_destroy(session), XENORA_OK);
    assert!(
        start.elapsed() < Duration::from_millis(500),
        "destroy took too long: {:?}",
        start.elapsed()
    );
}
