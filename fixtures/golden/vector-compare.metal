#include <metal_stdlib>
using namespace metal;
kernel void xenora_compute(
    device const uint* input [[buffer(0)]],
    device uint* output [[buffer(1)]],
    uint index [[thread_position_in_grid]],
    uint grid [[threads_per_grid]]) {
    uint t0 = input[index];
    uint t1 = 0x00000000u;
    uint t2 = 0x00000001u;
    uint t3 = 0x0f0f0f0fu;
    uint t4 = 0xa5a5a5a5u;
    uint t5 = 0xffffffffu;
    uint t6 = 0u;
    for (uint lane = 0u; lane < 32u; ++lane) {
        uint lane_index = (index & ~31u) + lane;
        if (lane_index >= grid) { break; }
        uint l0 = input[lane_index];
        uint pass = (as_type<int>(l0) < as_type<int>(t5)) ? 1u : 0u;
        t6 |= ((0xffffffffu >> lane) & pass) << lane;
    }
    uint t7 = t1 << (t2 & 31u);
    uint t8 = t7 ^ t6;
    uint t9 = 0u;
    for (uint lane = 0u; lane < 32u; ++lane) {
        uint lane_index = (index & ~31u) + lane;
        if (lane_index >= grid) { break; }
        uint l0 = input[lane_index];
        uint pass = (l0 < t5) ? 1u : 0u;
        t9 |= ((0xffffffffu >> lane) & pass) << lane;
    }
    uint t10 = t8 << (t2 & 31u);
    uint t11 = t10 ^ t9;
    uint t12 = 0x00000000u;
    uint t13 = 0u;
    for (uint lane = 0u; lane < 32u; ++lane) {
        uint lane_index = (index & ~31u) + lane;
        if (lane_index >= grid) { break; }
        uint l0 = input[lane_index];
        uint pass = (as_type<int>(t12) == as_type<int>(l0)) ? 1u : 0u;
        t13 |= ((0xffffffffu >> lane) & pass) << lane;
    }
    uint t14 = t11 << (t2 & 31u);
    uint t15 = t14 ^ t13;
    uint t16 = 0x80000000u;
    uint t17 = 0u;
    for (uint lane = 0u; lane < 32u; ++lane) {
        uint lane_index = (index & ~31u) + lane;
        if (lane_index >= grid) { break; }
        uint l0 = input[lane_index];
        uint pass = (t16 == l0) ? 1u : 0u;
        t17 |= ((0xffffffffu >> lane) & pass) << lane;
    }
    uint t18 = t15 << (t2 & 31u);
    uint t19 = t18 ^ t17;
    uint t20 = 0x7fffffffu;
    uint t21 = 0u;
    for (uint lane = 0u; lane < 32u; ++lane) {
        uint lane_index = (index & ~31u) + lane;
        if (lane_index >= grid) { break; }
        uint l0 = input[lane_index];
        uint pass = (as_type<int>(t20) <= as_type<int>(l0)) ? 1u : 0u;
        t21 |= ((0xffffffffu >> lane) & pass) << lane;
    }
    uint t22 = t19 << (t2 & 31u);
    uint t23 = t22 ^ t21;
    uint t24 = 0x7fffffffu;
    uint t25 = 0u;
    for (uint lane = 0u; lane < 32u; ++lane) {
        uint lane_index = (index & ~31u) + lane;
        if (lane_index >= grid) { break; }
        uint l0 = input[lane_index];
        uint pass = (t24 <= l0) ? 1u : 0u;
        t25 |= ((0xffffffffu >> lane) & pass) << lane;
    }
    uint t26 = t23 << (t2 & 31u);
    uint t27 = t26 ^ t25;
    uint t28 = 0xfffffff0u;
    uint t29 = 0u;
    for (uint lane = 0u; lane < 32u; ++lane) {
        uint lane_index = (index & ~31u) + lane;
        if (lane_index >= grid) { break; }
        uint l0 = input[lane_index];
        uint pass = (as_type<int>(t28) > as_type<int>(l0)) ? 1u : 0u;
        t29 |= ((0xffffffffu >> lane) & pass) << lane;
    }
    uint t30 = t27 << (t2 & 31u);
    uint t31 = t30 ^ t29;
    uint t32 = 0x00000040u;
    uint t33 = 0u;
    for (uint lane = 0u; lane < 32u; ++lane) {
        uint lane_index = (index & ~31u) + lane;
        if (lane_index >= grid) { break; }
        uint l0 = input[lane_index];
        uint pass = (t32 > l0) ? 1u : 0u;
        t33 |= ((0xffffffffu >> lane) & pass) << lane;
    }
    uint t34 = t31 << (t2 & 31u);
    uint t35 = t34 ^ t33;
    uint t36 = 0u;
    for (uint lane = 0u; lane < 32u; ++lane) {
        uint lane_index = (index & ~31u) + lane;
        if (lane_index >= grid) { break; }
        uint l0 = input[lane_index];
        uint pass = (as_type<int>(l0) != as_type<int>(l0)) ? 1u : 0u;
        t36 |= ((0xffffffffu >> lane) & pass) << lane;
    }
    uint t37 = 0u;
    for (uint lane = 0u; lane < 32u; ++lane) {
        uint lane_index = (index & ~31u) + lane;
        if (lane_index >= grid) { break; }
        uint l0 = input[lane_index];
        uint pass = (t36 != l0) ? 1u : 0u;
        t37 |= ((0xffffffffu >> lane) & pass) << lane;
    }
    uint t38 = t35 << (t2 & 31u);
    uint t39 = t38 ^ t37;
    uint t40 = 0x80000001u;
    uint t41 = 0u;
    for (uint lane = 0u; lane < 32u; ++lane) {
        uint lane_index = (index & ~31u) + lane;
        if (lane_index >= grid) { break; }
        uint l0 = input[lane_index];
        uint pass = (as_type<int>(t40) >= as_type<int>(l0)) ? 1u : 0u;
        t41 |= ((0xffffffffu >> lane) & pass) << lane;
    }
    uint t42 = t39 << (t2 & 31u);
    uint t43 = t42 ^ t41;
    uint t44 = 0u;
    for (uint lane = 0u; lane < 32u; ++lane) {
        uint lane_index = (index & ~31u) + lane;
        if (lane_index >= grid) { break; }
        uint l0 = input[lane_index];
        uint pass = (l0 >= t3) ? 1u : 0u;
        t44 |= ((0x55555555u >> lane) & pass) << lane;
    }
    uint t45 = ((0x55555555u >> (index & 31u)) & 1u) != 0u ? t44 : t4;
    uint t46 = t43 ^ t45;
    uint t48 = 0x00000000u;
    uint t49 = t46 ^ t48;
    uint t51 = 0u;
    for (uint lane = 0u; lane < 32u; ++lane) {
        uint lane_index = (index & ~31u) + lane;
        if (lane_index >= grid) { break; }
        uint l0 = input[lane_index];
        uint pass = (l0 > t3) ? 1u : 0u;
        t51 |= ((0xffffffffu >> lane) & pass) << lane;
    }
    uint t53 = t49 ^ t3;
    uint t54 = t51 != 0u ? t53 : t49;
    output[index] = t54;
}
