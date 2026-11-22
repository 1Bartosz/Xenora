# Public Direct3D 12 resource binding

Microsoft's D3D12 Resource Binding Functional Spec v1.24 describes descriptors, heaps, descriptor tables and root signatures. A descriptor table identifies a subrange of a heap. A root signature describes how shader resource declarations map to root arguments. In the supported direct root descriptor forms, a root descriptor carries a GPU virtual address with no view length. These rules are scoped to the documented D3D12 interface (CLAIM-D3D12-BINDING). [D3D12 Resource Binding Functional Spec](https://microsoft.github.io/DirectX-Specs/d3d/ResourceBinding.html)

Keep resource identity, byte range and access permission explicit in the fixture interface. API bindings and resource lifetimes can inform future design. The published Windows descriptor rules supply no reviewed console descriptor bytes or guest address translation for the initial slice (CLAIM-XBOX-ISA-ABI).
