# Xbox GDK research

Microsoft Learn's public [GDK index](https://learn.microsoft.com/en-us/gaming/gdk/?view=gdk-2610) exposes an Xbox graphics topic. The topic itself requires membership in an Xbox NDA developer program. The public GDK README describes the licensed Xbox Extensions and identifies the graphics driver and shader compiler as major differences between PC and console development. See CLAIM-XBOX-GDK-ACCESS. [Graphics access notice](https://learn.microsoft.com/en-us/gaming/gdk/docs/features/graphics/gc-graphics-toc?view=gdk-2610), [GDK README](https://github.com/microsoft/GDK/blob/d773744074fb757ef27b2356c9aef5497b14623b/README.MD#frequently-asked-questions)

The public samples provide examples at the application API boundary. These are static source observations. The samples require their own platform toolchain and were not built or run for Xenora.

| Example | Observed behavior | Evidence |
|---|---|---|
| SimpleDeviceAndSwapChain | Calls `D3D12XboxCreateDevice` and submits presentation through `PresentX`. | CLAIM-XBOX-DEVICE-SAMPLE |
| SimpleCompute | Submits compute work through a graphics command list or an asynchronous compute command list, with a dependency before the consuming draw. | CLAIM-XBOX-COMPUTE-SAMPLE |
| HlslCompile | Includes a Series X\|S path using an offline ACO-compiled pipeline. The sample deserializes the pipeline at runtime and allocates memory for shader instructions. Its README identifies the serialized pipeline state as GDK-version-dependent. | CLAIM-XBOX-ACO-SAMPLE |

The sample repository's `IntroGraphics` and `Graphics` directories contain examples for resource use, mesh shaders, ray tracing and indirect execution. This inventory establishes examples for public reading. An implementation would need an applicable specification and independent verification for the particular behavior. [IntroGraphics](https://github.com/microsoft/Xbox-GDK-Samples/tree/b1e77b6ac10f95b07296025fda171ca2ddc42dbe/Samples/IntroGraphics), [Graphics](https://github.com/microsoft/Xbox-GDK-Samples/tree/b1e77b6ac10f95b07296025fda171ca2ddc42dbe/Samples/Graphics)
