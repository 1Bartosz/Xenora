# Historical Apple GPU laboratory intake

The supplied Phase 01 brief records 89 passes across 92 named experiments, including four LLVM-only experiments. The counts combine multiple runs and targeted reruns. These are historical observations awaiting reproduction against project code.

| Local run ID | Checksum intake |
|---|---:|
| gpu-lab.BJI5pRmA | Manifest absent |
| gpu-lab.2gOe22wp | 45 verified |
| gpu-lab2.H3sdhW1s | 59 verified |
| gpu-lab3.I8lyHB6z | 96 verified |
| gpu-lab3-fix.kwdpxFc1 | 38 verified |
| gpu-lab4.RHjxHxWD | 225 verified |
| gpu-lab4.GpNvgPep | 61 verified |

The intake validated manifest path containment and all listed SHA-256 digests. Checksums establish integrity relative to the manifests. Source provenance and redistributability still require review. No historical executable was run during intake. The first Xenora fixture was independently assembled from new repository source.

The FP32 subnormal arithmetic discrepancies and NaN payload observation remain open. See the [risk register](../risk-register.md). The historical SPIRV-Cross result also needs a source-level bounds review before reuse.
