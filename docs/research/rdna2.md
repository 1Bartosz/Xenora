# RDNA 2 source notes

Sources were retrieved on 2026-11-22. AMD's RDNA 2 programming reference and
machine-readable ISA describe the AMD instruction set used by the synthetic gfx1030
fixture. Treat them as class B within the stated AMD target. Console-specific encoding,
descriptor layout, launch state and applicability remain unknown.

| Source | Version and scope | What it establishes |
| --- | --- | --- |
| [AMD RDNA 2 ISA Reference Guide](https://docs.amd.com/v/u/en-US/rdna2-shader-instruction-set-architecture) | AMD document 70648, 30 November 2020 | §3.6 for register classes, §6.2 for vector sources, §8.1 and §12.14 for indexed buffer addresses and MUBUF, §12.2 and §12.5 for waits and termination, §12.7–12.8 for vector addition and moves. The guide is used as a reference with no text copied. |
| [AMD machine-readable ISA](https://gpuopen.com/machine-readable-isa/) | RDNA 2 XML, release date 2026-02-20, schema 1.1.1, AMD Public Use, MIT | Format bits, instruction widths and opcodes for the selected variants. |
| [AMD XML schema documentation](https://github.com/GPUOpen-Tools/isa_spec_manager/blob/720736542c7177de7e7970dec53d6a4a72cfef52/documentation/spec_documentation.md) | Upstream commit `720736542c7177de7e7970dec53d6a4a72cfef52` | Describes instruction encodings and bit layouts. No code imported. |
| [LLVM AMDGPU backend reference](https://llvm.org/docs/AMDGPUUsage.html) and [LLVM 23.1.6 source](https://github.com/llvm/llvm-project/tree/5b10d2d74929eae78912771222c9855af97496f4) | Homebrew LLVM 23.1.6, upstream release commit `5b10d2d74929eae78912771222c9855af97496f4` | The native assembler/disassembler accepts the exact gfx1030 Wave32 forms listed in the coverage table. This is class E tool behavior. |

AMD §8.1 describes the index, stride, offsets and bounds rules of a hardware
descriptor. Xenora's fixture supplies two bindings in advance under [ADR
0003](../adr/0003-demo-memory-model.md). It does not decode descriptors. AMD
§12.5 defines separate vector-load and vector-store accounting and describes
implicit waits at termination (class B, CLAIM-RDNA2-TRANSFER-WAITS). The accepted wait operands are chosen narrowly
for the fixture. AMD §12.7 specifies a uint32 sum with no carry. The VOP3
encoding for this VOP2 operation is described in §12.7.1 and is identified
separately in AMD's XML.

## Table provenance

The [public AMD archive](https://gpuopen.com/download/machine-readable-isa/latest/)
provided `amdgpu_isa_rdna2.xml` with SHA-256
`d671ecbc36543674ab59e9b2feffd56718fd2137e7ad913c314f2e2508b9f4f7`.
The release date is from the XML metadata. AMD copyright and MIT licensing
cover the generated table, which reduces the XML data to first-word masks,
opcode patterns and base lengths. The Python generator accepts only this
exact XML revision and does no network access. AMD may update the archive at
the same URL, so preserve the accepted XML outside the repository for future
regeneration.

Run from the checkout root after extracting the approved XML.

```sh
python3 crates/isa/generate_rdna2.py path/to/amdgpu_isa_rdna2.xml | diff -u crates/isa/src/rdna2.rs -
```

## Complete instruction metadata

`crates/isa/generate_rdna2_meta.py` reduces the same approved XML to every
gfx1030 base instruction form: 1,460 forms in 21 encoding families, with
identifying bits, field layouts, operands (field, data format, operand type,
width, direction), control-flow flags and allowed extension words. It runs in
two stages so regeneration and CI checks need no network and no XML:

```sh
python3 crates/isa/generate_rdna2_meta.py --xml path/to/amdgpu_isa_rdna2.xml  # XML -> snapshot -> Rust
python3 crates/isa/generate_rdna2_meta.py --check                              # snapshot -> Rust, compare
```

The snapshot `crates/isa/data/rdna2.jsonl` is sorted, normalised MIT data
under the AMD notice in `crates/isa/LICENSE-AMD`. The generator rejects a
different XML hash, another architecture, unclassified encodings, duplicate
variants, identifying bits outside a mask and overlapping encodings. Two
details are not in the identifier masks and are handled explicitly: MIMG
opcode bit 7 sits at bit 0, and MTBUF opcode bit 3 sits at bit 53.

The XML names literal, SDWA, DPP8, DPP16 and NSA variants without a rule for
when they apply. `xenora_isa::meta::identify` implements those length rules by
hand: a source field equal to 255 (`SRC_LITERAL` in the XML) adds a literal;
VOP1, VOP2 and VOPC `SRC0` values 0xF9, 0xFA, 0xE9 and 0xEA add an SDWA, DPP16
or DPP8 control dword (from the RDNA 2 ISA guide's SDWA and DPP sections, not
the XML); MIMG `NSA` adds that many address dwords. An extension the form does
not list is rejected. Every LLVM-assembled fixture identifies with exact
boundaries and matching mnemonics. A broader LLVM comparison of the extension
rules is still open because it needs the LLVM oracle.

Identification is not execution. `xenora shaders coverage` reports forms
discovered in metadata against forms the semantic decoder accepts
(`xenora_decoder::SEMANTIC_FORMS`); reference, Metal and native GPU evidence
per form remains in `docs/status.toml`.
