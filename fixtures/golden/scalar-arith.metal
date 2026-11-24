#include <metal_stdlib>
using namespace metal;
kernel void xenora_compute(
    device const uint* input [[buffer(0)]],
    device uint* output [[buffer(1)]],
    uint index [[thread_position_in_grid]]) {
    uint t0 = input[index];
    uint t2 = 0x89abcdefu;
    uint t3 = 0x00000005u;
    uint t4 = t2 ^ t3;
    uint t5 = 0x89abcdefu;
    uint t6 = t4 ^ t5;
    uint t7 = 0x0000fff0u;
    uint t8 = t6 ^ t7;
    uint t9 = 0x13579be3u;
    uint t10 = t8 ^ t9;
    uint t11 = 0xb05b05abu;
    uint t12 = t10 ^ t11;
    uint t13 = 0x000000deu;
    uint t14 = t12 ^ t13;
    uint t15 = 0xffffffdeu;
    uint t16 = t14 ^ t15;
    uint t17 = 0x76543216u;
    uint t18 = t16 ^ t17;
    uint t19 = 0xd70a3d67u;
    uint t20 = t18 ^ t19;
    uint t21 = 0x00000040u;
    uint t22 = t20 ^ t21;
    uint t23 = 0x4a0955b6u;
    uint t24 = t22 ^ t23;
    uint t25 = 0x36b1b9d8u;
    uint t26 = t24 ^ t25;
    uint t27 = 0x00000005u;
    uint t28 = t26 ^ t27;
    uint t29 = 0x00000005u;
    uint t30 = t28 ^ t29;
    uint t31 = 0x00000019u;
    uint t32 = t30 ^ t31;
    uint t33 = 0x00000005u;
    uint t34 = t32 ^ t33;
    uint t35 = 0x00000000u;
    uint t36 = t34 ^ t35;
    uint t37 = t0 ^ t36;
    output[index] = t37;
}
