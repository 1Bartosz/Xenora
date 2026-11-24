#include <metal_stdlib>
using namespace metal;
kernel void xenora_compute(
    device const uint* input [[buffer(0)]],
    device uint* output [[buffer(1)]],
    uint index [[thread_position_in_grid]]) {
    uint t0 = input[index];
    uint t2 = 0xffffff0fu;
    uint t3 = 0x0f000000u;
    uint t4 = t2 ^ t3;
    uint t5 = 0x00000004u;
    uint t6 = t4 ^ t5;
    uint t7 = 0x0000001cu;
    uint t8 = t6 ^ t7;
    uint t9 = 0x00000016u;
    uint t10 = t8 ^ t9;
    uint t11 = 0x00000010u;
    uint t12 = t10 ^ t11;
    uint t13 = 0x00000018u;
    uint t14 = t12 ^ t13;
    uint t15 = 0x0000000cu;
    uint t16 = t14 ^ t15;
    uint t17 = 0xfffffff0u;
    uint t18 = t16 ^ t17;
    uint t19 = 0xffff8000u;
    uint t20 = t18 ^ t19;
    uint t21 = 0x00008000u;
    uint t22 = t20 ^ t21;
    uint t23 = 0x00007fffu;
    uint t24 = t22 ^ t23;
    uint t25 = 0xffffffffu;
    uint t26 = t24 ^ t25;
    uint t27 = t0 ^ t26;
    output[index] = t27;
}
