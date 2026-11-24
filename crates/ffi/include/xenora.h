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
};

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

#endif
