use xenora_metal::NativeCompute;

#[path = "native/guest_buffer.rs"]
mod guest_buffer;
#[path = "native/offscreen.rs"]
mod offscreen;
#[path = "native/texture.rs"]
mod texture;
#[path = "native/texture_bc_alpha.rs"]
mod texture_bc_alpha;
#[path = "native/texture_bc_channel.rs"]
mod texture_bc_channel;
#[path = "native/texture_sampling.rs"]
mod texture_sampling;
#[path = "native/translated_render.rs"]
mod translated_render;

const KERNEL_HEADER: &str = r"
#include <metal_stdlib>
using namespace metal;
kernel void fixture(device const uint *input [[buffer(0)]],
                    device uint *output [[buffer(1)]],
                    uint id [[thread_position_in_grid]],
                    uint local [[thread_index_in_threadgroup]],
                    uint group_size [[threads_per_threadgroup]]) {
";

fn fixture(body: &str) -> String {
    format!("{KERNEL_HEADER}{body}\n}}")
}

fn context() -> NativeCompute {
    NativeCompute::new().expect("native Metal device and queue")
}

fn verify_words(body: &str, input: &[u32], expected: &[u32]) {
    let actual = context()
        .run_words(&fixture(body), "fixture", input)
        .expect("compiled and completed native Metal dispatch");
    assert_eq!(actual, expected);
}

#[test]
fn integer_multiplication() {
    let context = context();
    println!(
        "device={} api_validation_requested={} shader_validation_requested={}",
        context.device_name(),
        std::env::var("MTL_DEBUG_LAYER").as_deref() == Ok("1"),
        std::env::var("MTL_SHADER_VALIDATION").as_deref() == Ok("1")
    );

    let values = [0, 1, 2, 3, u32::MAX, 0x8000_0000, 0x4000_0000];
    let input: Vec<_> = values.into_iter().cycle().take(131).collect();
    let expected: Vec<_> = input.iter().map(|&value| value.wrapping_mul(7)).collect();
    let actual = context
        .run_words(&fixture("output[id] = input[id] * 7u;"), "fixture", &input)
        .expect("compiled and completed native Metal dispatch");
    assert_eq!(actual, expected);
    println!(
        "checked {} output words in a nonuniform grid",
        expected.len()
    );
}

#[test]
fn vector_addition() {
    verify_words(
        "output[id] = input[id] + input[id ^ 1u];",
        &[u32::MAX, 2, 0x8000_0000, 0x8000_0001, 123, 42, 0, 0],
        &[1, 1, 1, 1, 165, 165, 0, 0],
    );
}

#[test]
fn bitwise_operations() {
    let input = [0, 1, u32::MAX, 0x8000_0000, 0x1122_3344, 0x5555_AAAA];
    let expected = input.map(|value| ((value & 0x0F0F_0000) >> 8) ^ (!value & 0xFF));
    verify_words(
        "output[id] = ((input[id] & 0x0f0f0000u) >> 8u) ^ (~input[id] & 0xffu);",
        &input,
        &expected,
    );
}

#[test]
fn copy_across_distinct_bindings() {
    let input = [0x0123_4567, 0, u32::MAX, 0x8000_0000, 0x89AB_CDEF];
    verify_words("output[id] = input[id];", &input, &input);
}

#[test]
fn exactly_representable_floating_point_arithmetic() {
    let input = [0.0_f32, 0.5, -0.5, 1.25, -2.0, 1024.0];
    let expected = [0.25_f32, 1.25, -0.75, 2.75, -3.75, 2048.25];
    verify_words(
        "output[id] = as_type<uint>(as_type<float>(input[id]) * 2.0f + 0.25f);",
        &input.map(f32::to_bits),
        &expected.map(f32::to_bits),
    );
}

#[test]
fn conditional_branching() {
    verify_words(
        r"
        if (input[id] > 10u) {
            output[id] = input[id] - 10u;
        } else {
            output[id] = input[id] * 4u;
        }
        ",
        &[0, 10, 11, 3, u32::MAX, 17],
        &[0, 40, 1, 12, u32::MAX - 10, 7],
    );
}

#[test]
fn synchronized_threadgroup_memory() {
    let input: Vec<_> = (100..228).collect();
    let expected: Vec<_> = (0..input.len()).map(|index| input[index ^ 1]).collect();
    verify_words(
        r"
        threadgroup uint stored[64];
        stored[local] = input[id];
        threadgroup_barrier(mem_flags::mem_threadgroup);
        uint neighbor = local ^ 1u;
        output[id] = neighbor < group_size ? stored[neighbor] : 0u;
        ",
        &input,
        &expected,
    );
}

#[test]
fn threadgroup_atomic_addition() {
    verify_words(
        r"
        threadgroup atomic_uint sum;
        if (local == 0u) {
            atomic_store_explicit(&sum, 0u, memory_order_relaxed);
        }
        threadgroup_barrier(mem_flags::mem_threadgroup);
        atomic_fetch_add_explicit(&sum, input[id], memory_order_relaxed);
        threadgroup_barrier(mem_flags::mem_threadgroup);
        output[id] = atomic_load_explicit(&sum, memory_order_relaxed);
        ",
        &[5, 0, 2, 1, 17, 4, 3, 8],
        &[40; 8],
    );
}

#[test]
fn simdgroup_reduction() {
    let input: Vec<_> = (0..128).map(|index| 1 + (index & 1)).collect();
    // The historical M5 probe measured 32 lanes. Every pair sums to 3,
    // so a complete 32-lane SIMD group of this input sums to 48.
    verify_words("output[id] = simd_sum(input[id]);", &input, &[48; 128]);
}

#[test]
fn multiple_submissions() {
    let context = context();
    let source = fixture("output[id] = input[id] + id;");
    let first = context.run_words(&source, "fixture", &[1, 2, 3]).unwrap();
    let second = context
        .run_words(&source, "fixture", &[5, 4, u32::MAX, 2])
        .unwrap();
    assert_eq!(first, [1, 3, 5]);
    assert_eq!(second, [5, 5, 1, 5]);
}

#[test]
fn errors_propagate() {
    let context = context();
    let source = fixture("output[id] = input[id];");
    let error = context
        .run_words("not valid MSL", "fixture", &[25])
        .expect_err("invalid source must fail");
    assert_eq!(error.operation, "newLibraryWithSource");
    assert!(!error.detail.is_empty());
    let error = context
        .run_words(&source, "nonexistent_entry", &[25])
        .expect_err("missing entry must fail");
    assert_eq!(error.operation, "newFunctionWithName");
    let error = context
        .run_words(&source, "fixture", &[])
        .expect_err("empty dispatch must fail");
    assert_eq!(error.operation, "dispatch bounds");
    let error = context
        .run_words(&source, "fixture", &[0; 4097])
        .expect_err("excessive dispatch must fail");
    assert_eq!(error.operation, "dispatch bounds");
}
