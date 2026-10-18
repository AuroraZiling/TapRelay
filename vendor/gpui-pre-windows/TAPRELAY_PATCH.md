# Default WARP rendering

TapRelay uses WARP software rendering on Windows to reduce idle memory usage.

This is the published `gpui-pre-windows` 0.3.9 source, under Apache-2.0.
See `LICENSE-APACHE` and the repository's `THIRD_PARTY_NOTICES.md`.

`DirectXRendererDevices::gpu_specs` obtains its metadata adapter from the created
D3D11 device. The requested adapter can differ when DXCPL forces WARP.

Device creation always uses a null adapter and `D3D_DRIVER_TYPE_WARP`, preserving
the upstream feature checks and shaders. Hardware adapter enumeration and the
experimental environment switch have been removed. Administrator relaunches and
device-loss recovery use the same default. Creation failures are reported
instead of silently falling back to hardware.
