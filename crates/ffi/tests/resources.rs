// SPDX-License-Identifier: GPL-3.0-or-later

//! Gate 5 acceptance on a native Metal device: persistent GPU resources,
//! dynamic texture updates, and completion-aware resource retirement through
//! the C interface (#279).
//! Needs `--features gpu-tests` (scripts/test-gpu.sh).
#![cfg(feature = "gpu-tests")]

use std::sync::Mutex;
use xenora_ffi::*;

/// Tests share the process-wide session table and one GPU; run them one at a time.
static SERIAL: Mutex<()> = Mutex::new(());

fn session() -> u64 {
    let mut handle = 0;
    assert_eq!(
        unsafe { xenora_session_create(XENORA_ABI_VERSION, &mut handle) },
        XENORA_OK
    );
    assert_ne!(handle, 0);
    handle
}

#[test]
fn persistent_texture_lifecycle_survives_across_multiple_frames() {
    let _guard = SERIAL.lock().unwrap();
    let sess = session();

    // 2x2 initial texture:
    // (0,0)=red, (1,0)=green, (0,1)=blue, (1,1)=white
    let initial_rgba: [u8; 16] = [
        255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255,
    ];

    let mut res = 0u64;
    assert_eq!(
        unsafe { xenora_session_create_texture(sess, 2, 2, initial_rgba.as_ptr(), &mut res) },
        XENORA_OK
    );
    assert_ne!(res, XENORA_NO_RESOURCE);

    // Render 5 consecutive frames from this single persistent texture resource without re-allocation.
    for frame_idx in 0..5 {
        let mut out_rgba = [0u8; 16];
        let mut out_w = 0u32;
        let mut out_h = 0u32;

        assert_eq!(
            unsafe {
                xenora_session_render_textured(
                    sess,
                    res,
                    out_rgba.as_mut_ptr(),
                    16,
                    &mut out_w,
                    &mut out_h,
                )
            },
            XENORA_OK,
            "render frame {} failed",
            frame_idx
        );

        assert_eq!(out_w, 2);
        assert_eq!(out_h, 2);
        assert_eq!(
            out_rgba, initial_rgba,
            "frame {} pixel mismatch with persistent texture",
            frame_idx
        );
    }

    // Verify resource is still live and un-retired after all frames
    let (mut r_type, mut r_w, mut r_h, mut r_flight) = (0u32, 0u32, 0u32, 0u32);
    assert_eq!(
        unsafe {
            xenora_session_resource_info(sess, res, &mut r_type, &mut r_w, &mut r_h, &mut r_flight)
        },
        XENORA_OK
    );
    assert_eq!(r_type, XENORA_RESOURCE_TYPE_TEXTURE_2D);
    assert_eq!(r_w, 2);
    assert_eq!(r_h, 2);
    assert_eq!(r_flight, 0);

    assert_eq!(xenora_session_destroy_resource(sess, res), XENORA_OK);
    assert_eq!(xenora_session_destroy(sess), XENORA_OK);
}

#[test]
fn dynamic_texture_subregion_update_is_reflected_on_metal() {
    let _guard = SERIAL.lock().unwrap();
    let sess = session();

    // 2x2 initial texture:
    // (0,0)=red, (1,0)=green, (0,1)=blue, (1,1)=white
    let initial_rgba: [u8; 16] = [
        255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255,
    ];

    let mut res = 0u64;
    assert_eq!(
        unsafe { xenora_session_create_texture(sess, 2, 2, initial_rgba.as_ptr(), &mut res) },
        XENORA_OK
    );

    // 1. Update 1x1 subregion at (x=1, y=0) from green to yellow [255, 255, 0, 255]
    let yellow_pixel = [255u8, 255, 0, 255];
    assert_eq!(
        unsafe { xenora_session_update_texture(sess, res, 1, 0, 1, 1, yellow_pixel.as_ptr(), 4) },
        XENORA_OK
    );

    let mut frame1 = [0u8; 16];
    let (mut w, mut h) = (0u32, 0u32);
    assert_eq!(
        unsafe {
            xenora_session_render_textured(sess, res, frame1.as_mut_ptr(), 16, &mut w, &mut h)
        },
        XENORA_OK
    );

    let expected_frame1: [u8; 16] = [
        255, 0, 0, 255, // (0,0) red (untouched)
        255, 255, 0, 255, // (1,0) yellow (updated)
        0, 0, 255, 255, // (0,1) blue (untouched)
        255, 255, 255, 255, // (1,1) white (untouched)
    ];
    assert_eq!(
        frame1, expected_frame1,
        "Metal read-back did not reflect 1x1 subregion update"
    );

    // 2. Update 2x1 subregion across the bottom row: (0,1) and (1,1)
    // (0,1)=magenta [255, 0, 255, 255], (1,1)=cyan [0, 255, 255, 255]
    let bottom_row = [255u8, 0, 255, 255, 0, 255, 255, 255];
    assert_eq!(
        unsafe { xenora_session_update_texture(sess, res, 0, 1, 2, 1, bottom_row.as_ptr(), 8) },
        XENORA_OK
    );

    let mut frame2 = [0u8; 16];
    assert_eq!(
        unsafe {
            xenora_session_render_textured(sess, res, frame2.as_mut_ptr(), 16, &mut w, &mut h)
        },
        XENORA_OK
    );

    let expected_frame2: [u8; 16] = [
        255, 0, 0, 255, // (0,0) red
        255, 255, 0, 255, // (1,0) yellow
        255, 0, 255, 255, // (0,1) magenta
        0, 255, 255, 255, // (1,1) cyan
    ];
    assert_eq!(
        frame2, expected_frame2,
        "Metal read-back did not reflect 2x1 bottom row update"
    );

    assert_eq!(xenora_session_destroy_resource(sess, res), XENORA_OK);
    assert_eq!(xenora_session_destroy(sess), XENORA_OK);
}

#[test]
fn deferred_resource_retirement_safely_waits_for_completion() {
    let _guard = SERIAL.lock().unwrap();
    let sess = session();

    let mut res = 0u64;
    assert_eq!(
        unsafe { xenora_session_create_texture(sess, 2, 2, std::ptr::null(), &mut res) },
        XENORA_OK
    );

    // Acquire an in-flight reference simulating active GPU encoding/submission
    assert_eq!(test_acquire_resource_in_flight(sess, res), Ok(()));

    let (mut r_type, mut r_w, mut r_h, mut r_flight) = (0u32, 0u32, 0u32, 0u32);
    assert_eq!(
        unsafe {
            xenora_session_resource_info(sess, res, &mut r_type, &mut r_w, &mut r_h, &mut r_flight)
        },
        XENORA_OK
    );
    assert_eq!(r_flight, 1);

    // Calling destroy while in-flight marks the resource retired without dangling pointers
    assert_eq!(xenora_session_destroy_resource(sess, res), XENORA_OK);

    // Subsequent caller operations on the retired handle are rejected
    assert_eq!(
        unsafe {
            xenora_session_resource_info(sess, res, &mut r_type, &mut r_w, &mut r_h, &mut r_flight)
        },
        XENORA_INVALID_HANDLE
    );
    assert_eq!(
        xenora_session_destroy_resource(sess, res),
        XENORA_INVALID_HANDLE
    );

    let dummy_pixel = [0u8; 4];
    assert_eq!(
        unsafe { xenora_session_update_texture(sess, res, 0, 0, 1, 1, dummy_pixel.as_ptr(), 4) },
        XENORA_INVALID_HANDLE
    );

    let mut out_rgba = [0u8; 16];
    let (mut w, mut h) = (0u32, 0u32);
    assert_eq!(
        unsafe {
            xenora_session_render_textured(sess, res, out_rgba.as_mut_ptr(), 16, &mut w, &mut h)
        },
        XENORA_INVALID_HANDLE
    );

    // Now in-flight execution completes and releases its reference
    assert_eq!(test_release_resource_in_flight(sess, res), Ok(()));

    // The slot is now safely reclaimed; the next allocation reuses the slot with a new generation
    let mut res2 = 0u64;
    assert_eq!(
        unsafe { xenora_session_create_texture(sess, 2, 2, std::ptr::null(), &mut res2) },
        XENORA_OK
    );
    assert_eq!((res & 0xFFFF_FFFF), (res2 & 0xFFFF_FFFF)); // same slot index
    assert_ne!(res >> 32, res2 >> 32); // higher generation

    assert_eq!(xenora_session_destroy_resource(sess, res2), XENORA_OK);
    assert_eq!(xenora_session_destroy(sess), XENORA_OK);
}

#[test]
fn invalid_stale_and_double_destroyed_resource_handles_are_rejected() {
    let _guard = SERIAL.lock().unwrap();
    let sess = session();

    // Zero handle
    assert_eq!(
        xenora_session_destroy_resource(sess, XENORA_NO_RESOURCE),
        XENORA_INVALID_HANDLE
    );

    // Unallocated slot index
    let unallocated_handle = (1u64 << 32) | 999u64;
    assert_eq!(
        xenora_session_destroy_resource(sess, unallocated_handle),
        XENORA_INVALID_HANDLE
    );

    // Create and destroy a valid resource
    let mut res = 0u64;
    assert_eq!(
        unsafe { xenora_session_create_texture(sess, 2, 2, std::ptr::null(), &mut res) },
        XENORA_OK
    );
    assert_eq!(xenora_session_destroy_resource(sess, res), XENORA_OK);

    // Double destroy
    assert_eq!(
        xenora_session_destroy_resource(sess, res),
        XENORA_INVALID_HANDLE
    );

    // Stale handle queries
    let (mut r_type, mut r_w, mut r_h, mut r_flight) = (0u32, 0u32, 0u32, 0u32);
    assert_eq!(
        unsafe {
            xenora_session_resource_info(sess, res, &mut r_type, &mut r_w, &mut r_h, &mut r_flight)
        },
        XENORA_INVALID_HANDLE
    );

    let pixel = [0u8; 4];
    assert_eq!(
        unsafe { xenora_session_update_texture(sess, res, 0, 0, 1, 1, pixel.as_ptr(), 4) },
        XENORA_INVALID_HANDLE
    );

    let mut out_rgba = [0u8; 16];
    let (mut w, mut h) = (0u32, 0u32);
    assert_eq!(
        unsafe {
            xenora_session_render_textured(sess, res, out_rgba.as_mut_ptr(), 16, &mut w, &mut h)
        },
        XENORA_INVALID_HANDLE
    );

    assert_eq!(xenora_session_destroy(sess), XENORA_OK);
}

#[test]
fn repeated_resource_allocations_and_retirements_do_not_leak_slots() {
    let _guard = SERIAL.lock().unwrap();
    let sess = session();

    let initial = [
        255u8, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255,
    ];
    let update = [255u8, 255, 0, 255];

    // Run 200 cycles of create -> update -> render -> destroy
    for i in 0..200 {
        let mut res = 0u64;
        assert_eq!(
            unsafe { xenora_session_create_texture(sess, 2, 2, initial.as_ptr(), &mut res) },
            XENORA_OK,
            "create failed at cycle {}",
            i
        );

        // Verify slot index is reused (res_idx is 0)
        let slot_idx = (res & 0xFFFF_FFFF) as usize;
        assert_eq!(slot_idx, 0, "slot index leaked at cycle {}", i);

        assert_eq!(
            unsafe { xenora_session_update_texture(sess, res, 0, 0, 1, 1, update.as_ptr(), 4) },
            XENORA_OK
        );

        let mut out = [0u8; 16];
        let (mut w, mut h) = (0u32, 0u32);
        assert_eq!(
            unsafe {
                xenora_session_render_textured(sess, res, out.as_mut_ptr(), 16, &mut w, &mut h)
            },
            XENORA_OK
        );

        assert_eq!(xenora_session_destroy_resource(sess, res), XENORA_OK);
    }

    assert_eq!(xenora_session_destroy(sess), XENORA_OK);
}

#[test]
fn textured_pe_fixture_executes_dynamic_updates_and_retirement() {
    let _guard = SERIAL.lock().unwrap();
    let sess = session();

    let mut out_rgba = [0u8; 16];
    let mut out_w = 0u32;
    let mut out_h = 0u32;

    assert_eq!(
        unsafe {
            xenora_session_run_textured_fixture(
                sess,
                out_rgba.as_mut_ptr(),
                16,
                &mut out_w,
                &mut out_h,
            )
        },
        XENORA_OK
    );

    assert_eq!(out_w, 2);
    assert_eq!(out_h, 2);

    let expected_final: [u8; 16] = [
        255, 255, 0, 255, // (0,0) yellow
        0, 255, 255, 255, // (1,0) cyan
        255, 0, 255, 255, // (0,1) magenta
        64, 64, 64, 255, // (1,1) dark grey
    ];
    assert_eq!(
        out_rgba, expected_final,
        "guest textured fixture read-back frame mismatch"
    );

    assert_eq!(xenora_session_destroy(sess), XENORA_OK);
}
