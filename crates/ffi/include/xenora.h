/* SPDX-License-Identifier: GPL-3.0-or-later */
/*
 * Xenora native application interface, version 1.
 *
 * Every function returns a xenora_status. Sessions are identified by opaque
 * 64-bit handles, never by pointers; 0 is never a valid handle. A destroyed
 * handle stays invalid even if its slot is reused. Output pointers are owned
 * by the caller and are written only when the call returns XENORA_OK.
 * All functions are thread-safe. No Rust panic crosses this boundary: one is
 * reported as XENORA_INTERNAL_ERROR.
 */
#ifndef XENORA_H
#define XENORA_H

#include <stdint.h>

#define XENORA_ABI_VERSION 1u

typedef int32_t xenora_status;
enum {
    XENORA_OK = 0,
    XENORA_INVALID_ARGUMENT = 1,
    XENORA_INVALID_HANDLE = 2,
    XENORA_VERSION_MISMATCH = 3,
    XENORA_INTERNAL_ERROR = 4,
    XENORA_PE_ERROR = 5,
    XENORA_LIMIT_EXCEEDED = 6,
    XENORA_BUFFER_TOO_SMALL = 7,
    XENORA_EXECUTION_FAILED = 8,
    XENORA_INVALID_STATE = 9,
};

/* Frame-loop states reported by xenora_session_frame_loop_status. */
enum {
    XENORA_LOOP_NONE = 0,
    XENORA_LOOP_RUNNING = 1,
    XENORA_LOOP_PAUSED = 2,
    XENORA_LOOP_FINISHED = 3,
    XENORA_LOOP_STOPPED = 4,
    XENORA_LOOP_FAILED = 5,
};

#define XENORA_FRAME_LOOP_MAX_FRAMES 100000u
/* Frame id reported when no frame has been completed yet. */
#define XENORA_NO_FRAME 0xFFFFFFFFu

/* Size of the frame drawn by the trusted fixture: RGBA8, rows top to bottom. */
#define XENORA_FIXTURE_FRAME_WIDTH 4u
#define XENORA_FIXTURE_FRAME_HEIGHT 3u

typedef uint64_t xenora_session;

/* The interface version this library implements. */
uint32_t xenora_abi_version(void);

/* Create a session. `abi_version` must equal XENORA_ABI_VERSION. */
xenora_status xenora_session_create(uint32_t abi_version, xenora_session *out_session);

/* Destroy a session. Destroying it again returns XENORA_INVALID_HANDLE. */
xenora_status xenora_session_destroy(xenora_session session);

/* Parse the bundled trusted PE fixture with the runtime loader. No guest code runs. */
xenora_status xenora_session_inspect_trusted_fixture(
    xenora_session session,
    uint32_t *out_section_count,
    uint32_t *out_entry_rva);

/*
 * Run the bundled trusted PE fixture: its x86-64 code is translated and executed,
 * and its single draw request is rendered on the native Metal device. On XENORA_OK,
 * `out_rgba` holds width * height * 4 bytes of the read-back frame, which has been
 * checked against the fixture shader's known output. `capacity` is the size of
 * `out_rgba` in bytes. Blocks the calling thread; call it off the main thread.
 * Only the embedded fixture can run; no caller-supplied executable is accepted.
 */
xenora_status xenora_session_run_trusted_fixture(
    xenora_session session,
    uint8_t *out_rgba,
    uint64_t capacity,
    uint32_t *out_width,
    uint32_t *out_height);

/*
 * Persistent rendering (one frame loop per session). Start runs the embedded
 * frame-loop fixture on a session-owned guest thread: the guest submits frames
 * 0, 1, 2, ... and each is rendered on native Metal and verified before it
 * becomes the latest frame. The guest stops after `frame_count` frames
 * (1..=XENORA_FRAME_LOOP_MAX_FRAMES). Pause holds the guest at its next frame
 * submission; resume continues it; stop makes it leave its loop and halt, and
 * waits for its thread. Destroying the session stops the loop first. Starting a
 * second loop on the same session returns XENORA_INVALID_STATE.
 */
xenora_status xenora_session_start_frame_loop(xenora_session session, uint32_t frame_count);
xenora_status xenora_session_pause_frame_loop(xenora_session session);
xenora_status xenora_session_resume_frame_loop(xenora_session session);
xenora_status xenora_session_stop_frame_loop(xenora_session session);

/*
 * Report the loop's state, the number of completed frames, and the latest
 * verified frame: its id (XENORA_NO_FRAME if none yet) and, when one exists,
 * its RGBA8 pixels (XENORA_FIXTURE_FRAME_WIDTH x HEIGHT) in `out_rgba`.
 */
xenora_status xenora_session_frame_loop_status(
    xenora_session session,
    uint32_t *out_state,
    uint32_t *out_completed,
    uint32_t *out_frame_id,
    uint8_t *out_rgba,
    uint64_t capacity);

/* Largest file xenora_inspect_executable accepts, in bytes. */
#define XENORA_MAX_INSPECT_BYTES 268435456u

/*
 * Read-only compatibility inspection of a caller-supplied PE file. Nothing in
 * `bytes` is executed: it is parsed and statically disassembled (bounded) only.
 * On XENORA_OK, XENORA_PE_ERROR or XENORA_INTERNAL_ERROR, `out_report` holds a
 * NUL-terminated UTF-8 report of `*out_required` bytes including the NUL.
 * XENORA_PE_ERROR means the file is not an acceptable PE32+ image;
 * XENORA_INTERNAL_ERROR means Xenora's own probe failed on it. The report says which. If `capacity`
 * is too small the call returns XENORA_BUFFER_TOO_SMALL with `*out_required`
 * set and nothing else written. Files over XENORA_MAX_INSPECT_BYTES return
 * XENORA_LIMIT_EXCEEDED.
 */
xenora_status xenora_inspect_executable(
    const uint8_t *bytes,
    uint64_t length,
    char *out_report,
    uint64_t capacity,
    uint64_t *out_required);

#endif
