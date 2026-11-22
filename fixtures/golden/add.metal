#include <metal_stdlib>
using namespace metal;
kernel void xenora_compute(
    device const uint* input [[buffer(0)]],
    device uint* output [[buffer(1)]],
    uint index [[thread_position_in_grid]]) {
    uint t0 = input[index];
    uint t1 = 0x12345678u;
    uint t2 = t0 + t1;
    output[index] = t2;
}
