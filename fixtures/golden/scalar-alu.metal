#include <metal_stdlib>
using namespace metal;
kernel void xenora_compute(
    device const uint* input [[buffer(0)]],
    device uint* output [[buffer(1)]],
    uint index [[thread_position_in_grid]]) {
    uint t0 = input[index];
    uint t2 = 0x7ffffffeu;
    uint t3 = 0xfffffff0u;
    uint t4 = t2 ^ t3;
    uint t5 = t0 ^ t4;
    output[index] = t5;
}
