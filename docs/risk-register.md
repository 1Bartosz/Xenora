# Open Phase 01 risks

| ID | Risk and impact | Current evidence | Owner and next experiment |
|---|---|---|---|
| R1 | Console shader container, launch state and resource descriptors remain unknown, so the fixture cannot establish console compatibility. | Public related-target specifications, class C | Xbox research. Seek an authorized exact-target specification before proposing a parser. |
| R2 | FP32 subnormal inputs can affect normal outputs. The historic V4 input-only probe reported four signed-zero mismatches. | Historical host observation, pending project reproduction | Metal and testing. Reproduce the original strict known answers in a bounded native Rust test. |
| R3 | FP32 subnormal outputs can be flushed even with normal inputs. The historic V4 output-only probe reported three signed-zero mismatches. | Historical host observation, pending project reproduction | Metal and testing. Reproduce both signs and the boundary rounding case. |
| R4 | The V3.1 exceptional arithmetic probe reported five subnormal mismatches and a canonical NaN payload. | Historical host observation, pending project reproduction | Compiler and reference. Keep the strict historical comparisons and establish the required guest policy before supporting float lowering. |
| R5 | Redistribution rights for SDK-derived Rust bindings and locked-version attribution require review before distributing linked binaries. | Pinned upstream license notices and maintainer comments | R1. Review the actual linked artifact and applicable terms. |
| R6 | The compiler skips unreachable instructions during verification. The reference also checks termination and the synthetic resource ABI on unreachable code, which can disagree with compiler acceptance. | R2 source inspection | Compiler and reference. Reproduce both cases and agree on a common acceptance policy. |
