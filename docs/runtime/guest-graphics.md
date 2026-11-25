# Guest graphics command path

Xenora's guest runtime can execute x86-64 programs, load original PE32+
executables, and translate gfx1030 vertex and pixel stages to Metal. What it
could not do, until this interface, was let a *guest program* author graphics
work. The D3D12-style runtime in `crates/loader/src/graphics.rs` offered
devices, command queues, allocators, command lists, resources, descriptor
heaps, root signatures, shader resource views, samplers, index buffers and
draws, but only as a Rust API that the guest could not reach.

The earlier prototype, `xenora_fixture.dll!SubmitGuestDraw`, did not close
that gap. It created its own device, queue, allocator and command list, read
one hardcoded 48-byte vertex buffer from a guest address, issued one
three-vertex draw with a caller-supplied clear colour, and returned a pixel
*count*. The guest could not create its own resources, bind its own index
buffer or render target, read a framebuffer back, or keep a device across
calls. A feature that exists inside the Metal backend but cannot be reached
from guest execution is not guest compatibility.

This document specifies the interface that closes it.

## Module and calling convention

The guest graphics path is exported from `xenora_graphics.dll`. It is a
separate module from `xenora_fixture.dll` so that the prototype services and
their existing fixtures keep working unchanged.

Every export uses the Win64 calling convention and returns an `HRESULT`
(`i32`) in `RAX`. `S_OK` is `0`. Failure codes are the usual Win32 values
mapped into `HRESULT` form, so `E_INVALIDARG` is `0x80070057` and `E_FAIL` is
`0x80004005`.

Guest object handles are opaque `u64` values. They are generation-checked, so a
handle of a destroyed object never resolves to a new object that reuses its
slot. Zero is not a valid handle. The guest passes a handle back exactly as it
received it; a handle is never a host pointer.

| Export | Arguments (Win64 order) | Effect |
| --- | --- | --- |
| `XenoraCreateDevice` | `ppDevice` | Creates a device and writes its handle to `*ppDevice`. |
| `XenoraCreateCommandQueue` | `device`, `listType`, `ppQueue` | Creates a command queue. `listType` must be `0` (direct). |
| `XenoraCreateCommandAllocator` | `device`, `listType`, `ppAllocator` | Creates a command allocator. `listType` must be `0`. |
| `XenoraCreateCommandList` | `device`, `allocator`, `listType`, `ppList` | Creates a recording command list. `listType` must be `0`. |
| `XenoraCreateBuffer` | `device`, `guestAddr`, `byteLength`, `ppResource` | Creates a buffer whose contents are the `byteLength` guest bytes at `guestAddr`. |
| `XenoraCreateRenderTarget` | `device`, `width`, `height`, `ppResource` | Creates an `R8G8B8A8_UNORM` two-dimensional render target. |
| `XenoraSetVertexBuffer` | `list`, `slot`, `resource`, `offset` | Binds a vertex buffer at `slot`. The stride is fixed at 16 bytes, one `float4` position per vertex. `slot` must be `0`. |
| `XenoraSetIndexBuffer` | `list`, `resource`, `offset`, `dxgiFormat` | Binds an index buffer. `dxgiFormat` is `57` (`R16_UINT`) or `42` (`R32_UINT`). |
| `XenoraSetRenderTarget` | `list`, `resource`, `clearColor` | Binds the colour target and the clear colour. `resource` may be `0` for no target. `clearColor` is packed `R | G<<8 | B<<16 | A<<24`. |
| `XenoraSetViewport` | `list`, `width`, `height` | Sets the viewport. Both dimensions must be in `1..=4096`. |
| `XenoraDraw` | `list`, `vertexCount`, `instanceCount`, `startVertex` | Records a non-indexed draw. |
| `XenoraDrawIndexed` | `list`, `indexCount`, `instanceCount`, `startIndexBase` | Records an indexed draw. `startIndexBase` packs `startIndex` in the low 32 bits and a sign-extended 16-bit `baseVertex` in the high 32 bits. |
| `XenoraCloseCommandList` | `list` | Ends recording so the list can be executed. |
| `XenoraExecuteCommandLists` | `queue`, `list` | Executes a closed list. The rendered pixels are written into the bound render target. |
| `XenoraReadbackResource` | `resource`, `guestAddr`, `byteLength` | Copies `byteLength` bytes of a resource into guest memory at `guestAddr`. |
| `XenoraGetLastFrameInfo` | `ppInfo` | Writes four `u32`s: backend, width, height, draw count. |
| `XenoraDestroyResource` | `resource` | Destroys a resource. |
| `XenoraDestroyCommandList` | `list` | Destroys a command list. |
| `XenoraDestroyCommandAllocator` | `allocator` | Destroys a command allocator. |
| `XenoraDestroyCommandQueue` | `queue` | Destroys a command queue. |
| `XenoraDestroyDevice` | `device` | Destroys a device. |

`XenoraGetLastFrameInfo` reports the backend that produced the last frame:
`0` for the deterministic software reference rasteriser and `1` for native
Metal. A guest that reports the wrong backend has not run on the GPU, and the
integration test checks this value rather than assuming it.

## Resource lifetime

Destruction is ordered, and the order is enforced rather than assumed.

- A device cannot be destroyed while a queue, allocator, command list or
  resource created from it is still alive.
- A command allocator cannot be destroyed while a command list created from it
  is still alive.
- A command list cannot be destroyed while it is still recording. Close it
  first.
- A resource cannot be destroyed while a recording command list references it.
- A command queue cannot be destroyed while a command list it submitted is
  still alive.

Each refusal is an explicit error, not a silent success and not a use-after-free.

## The execution path

A guest draw now travels the whole way:

```
guest PE32+ executable
  -> loader maps, relocates, binds imports
  -> guest calls xenora_graphics.dll exports
  -> services drive the GraphicsContext
  -> graphics commands are recorded and validated
  -> the backend renders (software reference, or native Metal)
  -> Metal runs the translated gfx1030 vertex and pixel stages
  -> pixels are written into the bound render target resource
  -> XenoraReadbackResource copies them into guest memory
  -> the guest inspects them and exits with a status
```

The native path crosses the existing `BackendRenderer` closure boundary, so
`crates/loader` stays free of a direct Metal dependency and the software
reference rasteriser remains the default on hosts without a GPU.

## Limits

- One triangle per draw, one instance, no primitive restart.
- One vertex layout: a `float4` position per vertex, stride 16.
- Render targets are `R8G8B8A8_UNORM`, at most 64 by 64.
- Descriptor heaps, root signatures, shader resource views and samplers exist
  in the Rust runtime but are not yet exported to the guest.
- Execution is synchronous. There is no guest-visible fence or queue
  overlap yet.
- The `x86_64` guest architecture is decoded by Xenora; the guest is not an
  Xbox title and no console behaviour is established here.

## Reproducing

The original guest program is
[`tests/compat_fixtures/guest_graphics.c`](../../tests/compat_fixtures/guest_graphics.c).
Its build script is
[`tests/compat_fixtures/build-guest-graphics.sh`](../../tests/compat_fixtures/build-guest-graphics.sh),
which cross-compiles it to x86-64 PE32+ and checks that the tools it uses are
native ARM64. The integration test is
[`crates/loader/tests/guest_graphics_path.rs`](../../crates/loader/tests/guest_graphics_path.rs):

```sh
cargo test --locked -p xenora-loader --test guest_graphics_path -- --nocapture
```

The native Metal variant needs a GPU and is run by `scripts/test-gpu.sh`.
