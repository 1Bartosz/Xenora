# Third-party notices

| Component | Version | License | Origin and notice |
|---|---|---|---|
| AMD RDNA 2 machine-readable ISA | 2026-02-20, schema 1.1.1 | MIT | Generated opcode masks derive from the AMD source identified in the [RDNA 2 research notes](docs/research/rdna2.md). [AMD license and copyright](crates/isa/LICENSE-AMD). |
| bitflags | 2.13.2 | MIT selected | [Upstream revision](https://github.com/bitflags/bitflags/tree/80ce9b545acb0bd42150695fc351889cac1d8eb4), [MIT license and copyright](licenses/bitflags-LICENSE-MIT). |
| objc2 | 0.6.5 | MIT | [Upstream revision](https://github.com/madsmtm/objc2/tree/d7d2fa23ceaa5e6096c923b081040e5d81b3b9df), [upstream licensing notice](licenses/objc2-LICENSE.md). |
| objc2-encode | 4.1.0 | MIT | [Upstream revision](https://github.com/madsmtm/objc2/tree/8d214f5477365ffcbcbb7de058c86ed9a518efb7), [upstream licensing notice](licenses/objc2-LICENSE.md). |
| objc2-foundation | 0.3.2 | MIT | [Upstream revision](https://github.com/madsmtm/objc2/tree/7b1abfd750a2cacaea71d6a56ecfb83cb7de560b), [upstream licensing notice](licenses/objc2-LICENSE.md). |
| objc2-metal | 0.3.2 | MIT selected | [Upstream revision](https://github.com/madsmtm/objc2/tree/7b1abfd750a2cacaea71d6a56ecfb83cb7de560b), [upstream licensing notice](licenses/objc2-LICENSE.md). |

Cargo resolves the Rust packages from crates.io at the source checksums in [Cargo.lock](Cargo.lock). The upstream objc2 notice discusses the derivation of bindings from Apple SDKs and an open question about redistribution. No Apple SDK files ship in this repository.

[LLVM 23.1.6](https://github.com/llvm/llvm-project/tree/5b10d2d74929eae78912771222c9855af97496f4) is used as an external fixture tool under Apache-2.0 WITH LLVM-exception. Xcode and Metal are separate Apple tools under Apple's license terms.

The [isolated fuzz workspace](fuzz/Cargo.lock) resolves developer-only dependencies, including [libfuzzer-sys](https://github.com/rust-fuzz/libfuzzer/tree/719e4efb9b8857ebaa782ae59376c8cbb78fed0f). See the [fuzz dependency inventory](docs/licenses.md#fuzz-development-dependencies) for locked revisions and selected licenses.
