# Native builds

[Back to the README](../README.md)

The default build compiles both the pinned VTK release and the C++ shim
statically. A resulting executable does not need VTK shared libraries.

## Build pipeline

1. [`vtk-wrap-sys/build.rs`](../vtk-wrap-sys/build.rs) downloads the pinned
   release archive, verifies its SHA-256, and unpacks it in the native cache.
2. CMake builds and installs VTK with `BUILD_SHARED_LIBS=OFF` and position
   independent code. Cargo features select the VTK modules to build, including
   their dependencies.
3. The shim is built against that VTK installation. The temporary
   `vtk_wrap_link_probe` executable verifies the native link and supplies a
   CMake link line that `build.rs` translates into Rust linker directives.

VTK is installed under
`<cache>/vtk-<version>/build/`; the default cache root is
`target/vtk-wrap-vtk/`. `VTK_WRAP_CACHE_DIR` overrides the root.
Build configuration stamps determine whether cached VTK can be reused.

The shim uses a separate `shim-<module-key>/` directory for each feature
set, rather than Cargo's per-run `OUT_DIR`. Its cache records the VTK
installation, target, generator, module set, and source hash so an unchanged
shim can be reused without recompiling all translation units.

## Local sources and prebuilt installations

`VTK_WRAP_VTK_SOURCE_DIR` supplies an unpacked VTK source tree; VTK is
still built locally. Use the pinned release that the committed bindings
target.

`VTK_WRAP_VTK_PREBUILT_DIR` supplies an install prefix, skipping the VTK
download and build. Its version must match the committed bindings, and it
must contain the enabled modules.

For static linking, the prebuilt VTK must have been configured with
`BUILD_SHARED_LIBS=OFF` and `CMAKE_POSITION_INDEPENDENT_CODE=ON`.
Shared installations can link too, but their shared libraries must be
available at runtime.

The environment-variable reference is in the
[README](../README.md#build-cache-and-overrides).

## Platform limitations

The committed sources were generated from macOS VTK headers. Known obstacles
to Linux and Windows builds include:

- Cocoa-specific classes and headers in the rendering modules.
- C++ array signatures using `int64_t` where VTK's `vtkIdType` has a
  different underlying type on other platforms.
- The native link reconstruction depends on CMake's `link.txt`, which the
  default Windows Visual Studio generator does not produce.

These limitations require platform-specific generation or native-build
changes; the [CI workflow](../.github/workflows/ci.yml) currently keeps Linux
and Windows builds non-blocking.
