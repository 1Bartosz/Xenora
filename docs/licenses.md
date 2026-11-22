# Dependency license decisions

The workspace is GPL-3.0-or-later. The following is the lead's preliminary compatibility inventory. The independent license audit is pending.

| Component | Version and upstream revision | License used | Scope |
|---|---|---|---|
| objc2 | 0.6.5, `d7d2fa23ceaa5e6096c923b081040e5d81b3b9df` | MIT | Runtime bindings |
| objc2-encode | 4.1.0, `8d214f5477365ffcbcbb7de058c86ed9a518efb7` | MIT | Runtime transitive dependency |
| objc2-foundation | 0.3.2, `7b1abfd750a2cacaea71d6a56ecfb83cb7de560b` | MIT | Foundation bindings |
| objc2-metal | 0.3.2, `7b1abfd750a2cacaea71d6a56ecfb83cb7de560b` | MIT from Zlib OR Apache-2.0 OR MIT | Metal bindings |
| bitflags | 2.13.2, `80ce9b545acb0bd42150695fc351889cac1d8eb4` | MIT from MIT OR Apache-2.0 | Runtime transitive dependency |
| AMD RDNA 2 ISA XML | 2026-02-20, schema 1.1.1 | MIT | Source for generated decoder constants |
| LLVM AMDGPU tools | 23.1.6 | Apache-2.0 WITH LLVM-exception | External fixture tool, not linked |
| Xcode and Metal tools | Xcode 27.2 | Apple proprietary terms | External host tools, not distributed |

MIT is compatible with the workspace license. The dependency revisions come from the registry packages' Cargo VCS metadata. Upstream notices and exact AMD source provenance are tracked separately.

The objc2 upstream notice raises an unresolved redistribution question about Apple SDK-derived bindings. Before distributing a linked build, resolve that question and obtain independent review. See the [preserved notice](../licenses/objc2-LICENSE.md).

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
