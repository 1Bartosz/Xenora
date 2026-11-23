// SPDX-License-Identifier: GPL-3.0-or-later
#include <metal_stdlib>
using namespace metal;

kernel void x3_or(device const uint *input [[buffer(0)]],
                  device uint *output [[buffer(1)]],
                  uint lane [[thread_position_in_grid]]) {
    output[lane] = input[lane] | 0x00ff00ffu;
}
