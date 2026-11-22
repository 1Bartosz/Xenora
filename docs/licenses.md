# Dependency license decisions

The workspace is GPL-3.0-or-later. The following is the lead's preliminary compatibility inventory. The independent license audit is pending.

| Component | Version and upstream revision | License used | Scope |
|---|---|---|---|
| objc2 | 0.6.5, `d7d2fa23ceaa5e6096c923b081040e5d81b3b9df` | MIT | Runtime bindings |
| objc2-encode | 4.1.0, `8d214f5477365ffcbcbb7de058c86ed9a518efb7` | MIT | Runtime transitive dependency |
| objc2-foundation | 0.3.2, `7b1abfd750a2cacaea71d6a56ecfb83cb7de560b` | MIT | Foundation bindings |
| objc2-core-foundation | 0.3.2, `7b1abfd750a2cacaea71d6a56ecfb83cb7de560b` | MIT from Zlib OR Apache-2.0 OR MIT | Core Foundation bindings |
| objc2-metal | 0.3.2, `7b1abfd750a2cacaea71d6a56ecfb83cb7de560b` | MIT from Zlib OR Apache-2.0 OR MIT | Metal bindings |
| bitflags | 2.13.2, `80ce9b545acb0bd42150695fc351889cac1d8eb4` | MIT from MIT OR Apache-2.0 | Runtime transitive dependency |
| AMD RDNA 2 ISA XML | 2026-02-20, schema 1.1.1 | MIT | Source for generated decoder constants |
| LLVM AMDGPU tools | 23.1.6 | Apache-2.0 WITH LLVM-exception | External fixture tool, not linked |
| Xcode and Metal tools | Xcode 27.2 | Apple proprietary terms | External host tools, not distributed |

## Runtime redistribution review

Reviewed 2026-11-22 against the locked Apple Silicon graph. Run `cargo tree --locked --target aarch64-apple-darwin --edges normal,build` and `cargo metadata --locked --format-version 1` to reproduce the graph and declared licenses. The external runtime graph comprises the six Cargo packages listed above. The reviewed registry packages identify their source revisions in `.cargo_vcs_info.json`. The five objc2 registry packages declare the listed licenses but contain no standalone license text. Each of the three pinned upstream `LICENSE.md` files matches the preserved copy byte for byte.

The declared MIT terms are compatible with GPL-3.0-or-later for code the upstream authors have rights to license. MIT remains the selected option for objc2-core-foundation, objc2-metal and bitflags. Distribution of copies or substantial portions subject to MIT requires retaining the applicable copyright and permission notice. Preserve the project GPL text, AMD's MIT notice, the bitflags MIT notice, and the objc2 notices. For a linked Xenora binary, also satisfy the GPLv3 section 6 Corresponding Source requirements for the shipped build, including applicable linked dependency source and build scripts, subject to the license's System Libraries exception. See the [project GPL text](../LICENSE).

The supplied objc2 MIT text comes from upstream commit `ee9a7ada2131f5944b8750428e265c15632f2a19`. Upstream confirms that the older crate versions use the license declared in their manifests, but did not confirm that the added copyright notice covers all earlier contributions. The text is preserved as upstream supplied it. Attribution completeness for the locked releases still requires review of applicable earlier notices. [Upstream MIT text](https://github.com/madsmtm/objc2/blob/ee9a7ada2131f5944b8750428e265c15632f2a19/LICENSE-MIT.txt) · [Maintainer discussion](https://github.com/madsmtm/objc2/issues/826)

The source checkout references the crates through Cargo and contains no dependency archives or Apple SDK files. A linked macOS build can incorporate generated Rust binding code even when Apple frameworks are linked dynamically. Apple's Xcode agreement §2.4 permits distribution of macOS applications and libraries subject to the agreement. §2.7 restricts redistribution and derivative works of Apple Software, with stated exceptions. The pinned objc2 notice and maintainer clarification leave open how those terms apply to generated bindings in redistributed source or linked binaries. G11 remains open pending review of that question for the actual distributable and independent confirmation of attribution. [Apple's Xcode agreement](https://www.apple.com/legal/sla/docs/xcode.pdf) · [Pinned objc2 notice](https://github.com/madsmtm/objc2/blob/7b1abfd750a2cacaea71d6a56ecfb83cb7de560b/LICENSE.md) · [Maintainer clarification](https://github.com/madsmtm/objc2/issues/895)

The lockfile also lists optional dispatch2 0.3.1 (revision `8852b424193ca41602281b3d7540d7c8ed51e49a`, MIT selected). It is absent from the native normal/build graph, including with all workspace features enabled.

On 2026-11-22 the objc2 maintainer explained that in their view the SDK-derived Foundation wrappers inline and leave calls to Apple functions in the final binary. Applicability to the linked Xenora artifact and downstream redistribution rights remain open. [Maintainer follow-up](https://github.com/madsmtm/objc2/issues/895#issuecomment-7079277040)

## Fuzz development dependencies

These packages are resolved only by the isolated fuzz workspace. Use MIT when it is an offered choice. The bundled LLVM libFuzzer source in `libfuzzer-sys` also carries NCSA terms. Fuzz executables are local test artifacts.

| Component | Locked version | Upstream revision | Selected license |
|---|---|---|---|
| arbitrary | 1.5.0 | `9807b599146c301a67e20692635352c9db70bff2` | MIT |
| cc | 1.11.3 | `c548c664cb6d71692959c1bd98959adc52ef7da7` | MIT |
| cfg-if | 1.0.5 | `8b0a2b0fc1fa3b3741b9e72895f390cf499fe36f` | MIT |
| find-msvc-tools | 0.1.15 | `ffbb10da608f715ed69a4561c4cbdca7c6fbbba6` | MIT |
| getrandom | 0.4.3 | `5e7cd5733536844a9856dc7259bd4696bbe5e3ae` | MIT |
| jobserver | 0.1.35 | `85ba04e3cf89e5c10e7b82de69f6be6dff9d3ec3` | MIT |
| libc | 0.2.191 | `ccc2ff4aaab962f7fde7b8edda7379c045b1ca12` | MIT |
| libfuzzer-sys | 0.4.13 | `719e4efb9b8857ebaa782ae59376c8cbb78fed0f` | MIT AND NCSA |
| r-efi | 6.0.0 | `7e1b0322d31d625f81a5656096330934f9cd835d` | MIT |
| shlex | 2.0.1 | `e82b1411beb7c92871c2c078c9ab415bbcf207ef` | MIT |
