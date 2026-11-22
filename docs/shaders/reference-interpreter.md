# Independent gfx1030 CPU reference

`xenora_ref::execute` reads raw little-endian code bytes and input words without dependencies on the production ISA crate, the compiler or Metal. It accepts at most 1024 instruction bytes and 4096 input words. The synthetic ABI gives every invocation its own input and output word. The indexed load and store forms need no host address arithmetic, and V0 is reserved for that indexing. The reference accepts uniform forward branches driven by scalar register state. Cross-lane register access and wave-mask changes are outside the subset. Floating-point arithmetic is restricted to positive normal inputs and exact normal results.

| AMD RDNA 2 ISA guide instruction | Reference operation |
|---|---|
| MUBUF `BUFFER_LOAD_DWORD`, `BUFFER_STORE_DWORD` | Track a pending read and write. Each accesses one 32-bit word under the synthetic mapping. Only one output store is allowed. |
| SOPP `S_WAITCNT`, SOPK `S_WAITCNT_VSCNT` | A zero vector-load or vector-store threshold completes the associated transfer. The model permits one pending operation per counter. |
| VOP1 `V_MOV_B32` | Read a register, inline integer or literal. Copy all 32 bits. |
| VOP3 `V_ADD_NC_U32` | Read two vector registers and add modulo 2^32. No implicit carry or flag update. |
| SOP1 `S_MOV_B32`, SOPC `S_CMP_EQ_U32` | Copy 32 bits into a scalar register or compare two operands and set SCC. Descriptor SGPRs are reserved. |
| SOPP `S_BRANCH`, `S_CBRANCH_SCC0` | Use the instruction following the branch as the displacement base. Require a nonnegative word displacement to a decoded instruction boundary, including for untaken edges. |
| VOP2 `V_AND_B32`, `V_XOR_B32` | Apply the bitwise operator to two VGPRs. |
| VOP2 `V_ADD_F32`, `V_MUL_F32` | Require exact binary32 results by checking the inputs and the wider intermediate. Multiplication of the significands fits in binary64. Addition must also recover the smaller operand by subtracting the larger. |
| SOPP `S_ENDPGM` | Stop at the end of the byte stream. The synthetic contract requires completed transfers and a produced output first. |

The encoding and instruction semantics are based on AMD's RDNA 2 ISA reference guide 70648, dated 30 November 2020, sections “Instruction Encoding” and “Microcode Formats.” Evidence is class C for applicability to Xbox hardware. The exact gfx1030 encodings, including VGPR 255 and VOP3 source-field crossings, were checked with LLVM 23.1.6 `llvm-mc` (class E). The project uses the document as a specification reference and distributes no specification text or LLVM code. [AMD RDNA 2 ISA reference](https://www.amd.com/content/dam/amd/en/documents/radeon-tech-docs/instruction-set-architectures/rdna2-shader-instruction-set-architecture.pdf)

Run `cargo test -p xenora-ref` for fixture results, both conditional paths, invalid targets, the exact FP32 domain, truncation, wait enforcement and register-index boundaries. This author-run suite is development evidence. Independent validation and three-way decode agreement are separately tracked release gates.
