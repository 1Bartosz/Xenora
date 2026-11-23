// SPDX-License-Identifier: GPL-3.0-or-later

use xenora_metal::{GuestBuffer, GuestBufferView, NativeCompute};
use xenora_shader::{Architecture, ENTRY_POINT, compile};

const CODE: &[u8] = include_bytes!("../fixtures/guest-buffer-xor.bin");
const INPUT: GuestBufferView = GuestBufferView {
    handle: 1,
    binding: 0,
    byte_offset: 4,
    byte_len: 16,
};
const PAYLOAD: &[u8] = &[
    0xde, 0xad, 0xbe, 0xef, // prefix outside the view
    0x00, 0x00, 0x00, 0x00, // 0x00000000
    0xff, 0xff, 0xff, 0xff, // 0xffffffff
    0x44, 0x33, 0x22, 0x11, // 0x11223344
    0x04, 0x03, 0x02, 0x01, // 0x01020304
    0xca, 0xfe, 0xf0, 0x0d, // suffix outside the view
];

#[test]
fn translated_shader_reads_validated_guest_buffer() {
    let context = NativeCompute::new().expect("native Metal device and queue");
    let shader = compile(Architecture::Gfx1030, CODE).expect("translate original gfx1030 shader");
    let table = [
        GuestBuffer {
            format: GuestBuffer::U32_LE,
            bytes: &[0x55; PAYLOAD.len()],
        },
        GuestBuffer {
            format: GuestBuffer::U32_LE,
            bytes: PAYLOAD,
        },
    ];
    let readback = context
        .run_guest_words(&shader.msl, ENTRY_POINT, &table, INPUT)
        .expect("complete translated Metal dispatch");
    let bytes: Vec<_> = readback.into_iter().flat_map(u32::to_le_bytes).collect();

    // Independent known answers: XOR each selected guest word with 0x01020304.
    assert_eq!(
        bytes,
        [
            0x04, 0x03, 0x02, 0x01, 0xfb, 0xfc, 0xfd, 0xfe, 0x40, 0x30, 0x20, 0x10, 0x00, 0x00,
            0x00, 0x00,
        ]
    );
    println!("Translated guest buffer readback: {bytes:02x?}");
}

#[test]
fn rejects_invalid_guest_buffer_before_compilation() {
    let context = NativeCompute::new().expect("native Metal device and queue");
    let oversized = [0; 16_388];
    let table = [
        GuestBuffer {
            format: 99,
            bytes: PAYLOAD,
        },
        GuestBuffer {
            format: GuestBuffer::U32_LE,
            bytes: PAYLOAD,
        },
        GuestBuffer {
            format: GuestBuffer::U32_LE,
            bytes: &oversized,
        },
    ];
    let cases = [
        (
            GuestBufferView {
                binding: 1,
                ..INPUT
            },
            "guest buffer binding",
        ),
        (
            GuestBufferView { handle: 3, ..INPUT },
            "guest buffer handle",
        ),
        (
            GuestBufferView { handle: 0, ..INPUT },
            "guest buffer format",
        ),
        (
            GuestBufferView {
                byte_offset: 1,
                ..INPUT
            },
            "guest buffer bounds",
        ),
        (
            GuestBufferView {
                byte_len: 3,
                ..INPUT
            },
            "guest buffer bounds",
        ),
        (
            GuestBufferView {
                byte_len: 0,
                ..INPUT
            },
            "guest buffer bounds",
        ),
        (
            GuestBufferView {
                handle: 2,
                byte_offset: 0,
                byte_len: oversized.len(),
                ..INPUT
            },
            "guest buffer bounds",
        ),
        (
            GuestBufferView {
                byte_offset: 12,
                ..INPUT
            },
            "guest buffer bounds",
        ),
        (
            GuestBufferView {
                byte_offset: usize::MAX & !3,
                ..INPUT
            },
            "guest buffer bounds",
        ),
    ];
    for (view, operation) in cases {
        let error = context
            .run_guest_words("invalid MSL", ENTRY_POINT, &table, view)
            .expect_err("guest view must fail before Metal compilation");
        assert_eq!(error.operation, operation, "{view:?}");
    }
}
