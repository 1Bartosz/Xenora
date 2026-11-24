// SPDX-License-Identifier: GPL-3.0-or-later

//! Runs the trusted fixture through the C interface on a native Metal device.
//! Needs `--features gpu-tests` (scripts/test-gpu.sh).
#![cfg(feature = "gpu-tests")]

use xenora_ffi::*;

/// fixtures/rdna2/x9-constant-ps.s: `v_mov_b32 v8, 1.0; v_mov_b32 v9, 0;
/// exp mrt0, v8, v9, v9, v8` writes (1, 0, 0, 1); as RGBA8 unorm that is (255, 0, 0, 255).
const SHADER_OUTPUT: [u8; 4] = [255, 0, 0, 255];

#[test]
fn guest_fixture_frame_reaches_the_caller_through_metal() {
    for run in 0..3 {
        let mut session = 0;
        assert_eq!(
            unsafe { xenora_session_create(XENORA_ABI_VERSION, &mut session) },
            XENORA_OK
        );
        let (mut rgba, mut width, mut height) = ([0u8; 64], 0u32, 0u32);
        let status = unsafe {
            xenora_session_run_trusted_fixture(
                session,
                rgba.as_mut_ptr(),
                64,
                &mut width,
                &mut height,
            )
        };
        assert_eq!(status, XENORA_OK, "run {run}");
        assert_eq!((width, height), (4, 3));
        assert_eq!(&rgba[..48], SHADER_OUTPUT.repeat(12).as_slice());
        assert!(
            rgba[48..].iter().all(|&b| b == 0),
            "no write past the frame"
        );
        assert_eq!(xenora_session_destroy(session), XENORA_OK);
    }
}
