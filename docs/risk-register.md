# Open Phase 01 risks

| ID | Risk and impact | Current evidence | Owner and next experiment |
|---|---|---|---|
| R1 | Console shader container, launch state and resource descriptors remain unknown, so the fixture cannot establish console compatibility. | Public related-target specifications, class C | Xbox research. Seek an authorized exact-target specification before proposing a parser. |
| R2 | FP32 subnormal inputs can affect normal outputs. The historic V4 input-only probe reported four signed-zero mismatches. | Historical host observation, pending project reproduction | Metal and testing. Reproduce the original strict known answers in a bounded native Rust test. |
| R3 | FP32 subnormal outputs can be flushed even with normal inputs. The historic V4 output-only probe reported three signed-zero mismatches. | Historical host observation, pending project reproduction | Metal and testing. Reproduce both signs and the boundary rounding case. |
| R4 | The V3.1 exceptional arithmetic probe reported five subnormal mismatches and a canonical NaN payload. | Historical host observation, pending project reproduction | Compiler and reference. Keep the strict historical comparisons and establish the required guest policy before supporting float lowering. |
