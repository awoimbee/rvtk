# Regenerating bindings

[Back to the README](../README.md)

The committed bindings target the VTK version pinned in
[`vtk-wrap-sys/build.rs`](../vtk-wrap-sys/build.rs). Users building the
crates do not need the generator or WrapVTK.

## From the pinned VTK

Build VTK first, then regenerate:

```sh
cargo build -p vtk-wrap-sys
./scripts/regenerate.sh
```

If using a prebuilt VTK, make sure it includes the `WrappingTools`
component and point the generator at its CMake package directory with
`VTK_DIR`. The build-script override `VTK_WRAP_VTK_PREBUILT_DIR` does
not itself configure the generator's lookup.

The generator searches `--vtk-dir`, `VTK_DIR`, the native cache, and
finally system installations, in that order.

[`vtk-wrap-gen`](../vtk-wrap-gen/src/main.rs) clones or reuses
[WrapVTK](https://github.com/dgobbi/WrapVTK), builds its `vtkWrapXML` tool,
and emits the C++ shim, raw FFI declarations, Rust wrappers, and smoke tests.
Generated files are overwritten; make binding changes in the generator.

## Module selection

By default, the module list comes from the `vtk*` features declared in
[`vtk-wrap-sys/Cargo.toml`](../vtk-wrap-sys/Cargo.toml), not just the
features enabled for a particular Cargo build.

To select modules explicitly:

```sh
MODULES=vtkCommonCore,vtkFiltersSources,vtkRenderingCore ./scripts/regenerate.sh
```

If you change the module set, keep the manifests' feature definitions aligned
with the generated modules.

## Generator options

List the CLI options rather than relying on a separate option table:

```sh
cargo run -p vtk-wrap-gen -- --help
```

For direct use, `cargo run -p vtk-wrap-gen -- --repo .` performs the same
generation pipeline. `--xml-dir` accepts existing WrapVTK XML and skips
building the wrapping tool; also supply `--vtk-include` when needed to
identify wrapper-only array shims.
