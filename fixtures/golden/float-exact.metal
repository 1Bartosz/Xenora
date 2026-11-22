#include <metal_stdlib>
using namespace metal;
kernel void xenora_compute(
    device const uint* input [[buffer(0)]],
    device uint* output [[buffer(1)]],
    uint index [[thread_position_in_grid]]) {
    #pragma clang fp contract(off)
    #pragma clang fp reassociate(off)
    uint t0 = input[index];
    uint t1 = 0x00400000u;
    uint t2 = t0 & t1;
    uint t3 = 0x3f800000u;
    uint t4 = t2 ^ t3;
    uint t5 = 0x40200000u;
    uint t6 = as_type<uint>(as_type<float>(t4) + as_type<float>(t5));
    uint t7 = as_type<uint>(as_type<float>(t6) * as_type<float>(t5));
    output[index] = t7;
}
