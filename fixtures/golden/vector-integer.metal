#include <metal_stdlib>
using namespace metal;
kernel void xenora_compute(
    device const uint* input [[buffer(0)]],
    device uint* output [[buffer(1)]],
    uint index [[thread_position_in_grid]]) {
    uint t0 = input[index];
    uint t1 = 0xfffffffdu;
    uint t2 = as_type<uint>((as_type<int>(t1 << 8u) >> 8)) * as_type<uint>((as_type<int>(t0 << 8u) >> 8));
    uint t3 = 0x00800001u;
    uint t4 = as_type<uint>(mulhi((as_type<int>(t3 << 8u) >> 8), (as_type<int>(t0 << 8u) >> 8)));
    uint t5 = 0x00123456u;
    uint t6 = (t5 & 0x00ffffffu) * (t0 & 0x00ffffffu);
    uint t7 = 0x00800001u;
    uint t8 = mulhi((t7 & 0x00ffffffu), (t0 & 0x00ffffffu));
    uint t9 = as_type<uint>(min(as_type<int>(t0), as_type<int>(t2)));
    uint t10 = 0x00000005u;
    uint t11 = as_type<uint>(max(as_type<int>(t10), as_type<int>(t0)));
    uint t12 = min(t4, t0);
    uint t13 = 0x00800001u;
    uint t14 = max(t13, t0);
    uint t15 = ~(t6 ^ t8);
    uint t16 = t9 + t11;
    uint t17 = t12 - t14;
    uint t18 = t16 - t15;
    uint t19 = t2 ^ t4;
    uint t20 = t6 ^ t19;
    uint t21 = t8 ^ t20;
    uint t22 = t9 ^ t21;
    uint t23 = t11 ^ t22;
    uint t24 = t12 ^ t23;
    uint t25 = t14 ^ t24;
    uint t26 = t15 ^ t25;
    uint t27 = t16 ^ t26;
    uint t28 = t17 ^ t27;
    uint t29 = t18 ^ t28;
    output[index] = t29;
}
