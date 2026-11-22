#include <metal_stdlib>
using namespace metal;
kernel void xenora_compute(
    device const uint* input [[buffer(0)]],
    device uint* output [[buffer(1)]],
    uint index [[thread_position_in_grid]]) {
    uint t0 = input[index];
    uint t2 = 0x00ff00ffu;
    uint t3 = t0 & t2;
    uint t4 = 0x12345678u;
    uint t5 = t3 ^ t4;
    output[index] = t5;
}
