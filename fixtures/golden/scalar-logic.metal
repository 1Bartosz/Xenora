#include <metal_stdlib>
using namespace metal;
kernel void xenora_compute(
    device const uint* input [[buffer(0)]],
    device uint* output [[buffer(1)]],
    uint index [[thread_position_in_grid]]) {
    uint t0 = input[index];
    uint t2 = 0xff0fffffu;
    uint t3 = 0x787ffff8u;
    uint t4 = t2 ^ t3;
    uint t5 = 0xffffffffu;
    uint t6 = t4 ^ t5;
    uint t7 = 0x00f00000u;
    uint t8 = t6 ^ t7;
    uint t9 = 0xf000ff00u;
    uint t10 = t8 ^ t9;
    uint t11 = t0 ^ t10;
    output[index] = t11;
}
