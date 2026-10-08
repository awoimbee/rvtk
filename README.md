# rvtk — Rust bindings for VTK

Safe-ish, thin, automatically generated Rust bindings for the
[Visualization Toolkit (VTK)](https://vtk.org/).

The bindings are generated from VTK's own wrapping metadata, so the wrapped API
tracks the installed VTK version instead of having to be written by hand.  The
generated surface is deliberately low level: it mirrors VTK rather than hiding
it.  Higher level, `pyvista`-style helpers belong in a separate crate.

* License: [BSD-3-Clause](LICENSE), the same license as VTK itself.
* Wrapped today: the `Common*` modules plus `FiltersCore`, `FiltersSources`,
  `FiltersGeneral`, `FiltersGeometry`, `IOGeometry`, `IOCore`, `IOLegacy`,
  `IOPLY` and `ImagingHybrid` — **1079 classes and ~21 300 methods**.
* The module list is configurable; see [Regenerating the bindings](#regenerating-the-bindings).

```rust
use vtk::vtkSphereSource;

let sphere = vtkSphereSource::new();
sphere.set_radius(2.0);
sphere.set_theta_resolution(32);
sphere.update();

let polydata = sphere.get_output().expect("output");
let points = polydata.get_points().expect("points");
println!("{} points", points.get_number_of_points());
```

## Requirements

* A C++ compiler and CMake (`cmake` ≥ 3.12).
* Network access the first time (to download the VTK source release), or a
  VTK source tree pointed at by `RVTK_VTK_SOURCE_DIR`.
* No installed VTK is used: `rvtk-sys` builds VTK from source and links it
  statically (see [Building](#building)).

## Workspace layout

| Crate | Lib name | Purpose |
| ----- | -------- | ------- |
| `rvtk` | `vtk` | Safe wrappers: one `#[repr(transparent)]` newtype per class, reference counting, inheritance via `Deref`. |
| `rvtk-sys` | `rvtk_sys` | Raw `extern "C"` declarations (`rvtk_sys::ffi`) and the CMake project that builds the C++ shim. |
| `rvtk-gen` | — | The code generator.  Consumes WrapVTK XML, emits the C++ shim, the FFI crate and the wrapper crate. |

> The *package* is called `rvtk` (the crate name `vtk` is taken on crates.io), but
the *library* is named `vtk`, so downstream code still reads
`use vtk::vtkSphereSource;`.

```
VTK headers
   │  vtkWrapXML  (WrapVTK)
   ▼
per-class XML ──► rvtk-gen ──┬─► rvtk-sys/shim/**.cpp  ──► librvtk_shim.a
                             ├─► rvtk-sys/src/generated.rs
                             └─► rvtk/src/generated.rs
```

## Building

```sh
cargo build -p rvtk-sys -p rvtk
cargo test  -p rvtk
cargo run   -p rvtk --example sphere_source
```

`rvtk-sys/build.rs` builds VTK from source, statically, then configures
`rvtk-sys/shim` against it with CMake and links the resulting `librvtk_shim.a`
into the crate.  The shim is a thin `extern "C"` layer: every function casts the
opaque receiver back to its concrete type and performs the call, so the code the
C++ compiler sees is essentially identical to hand written code.

### How VTK is built

The first build is expensive; everything after it is incremental.

1. The pinned VTK release (see `VTK_VERSION` in `rvtk-sys/build.rs`, currently
   9.7.1) is downloaded from `vtk.org` and checked against a SHA-256, then
   unpacked into `target/rvtk-vtk/vtk-<version>/`.
2. VTK is configured with `BUILD_SHARED_LIBS=OFF` and the components in
   `rvtk-sys/shim/CMakeLists.txt`, then built and installed into
   `target/rvtk-vtk/vtk-<version>/build/`.  A stamp file records the exact
   options, so a cached VTK is reused until they change.
3. The shim and a throw-away `rvtk_link_probe` executable are built against that
   static VTK.  `build.rs` reads the probe's CMake link line and turns it into
   `cargo:rustc-link-lib` directives, which is how the dozens of static VTK
   archives, system libraries and frameworks reach the Rust linker.  The shim's
   CMake build tree lives in `target/rvtk-vtk/vtk-<version>/shim/` (not in
   Cargo's per-run `OUT_DIR`), keyed by the VTK build, target, generator and a
   content hash of `rvtk-sys/shim`.  An unchanged shim therefore costs a no-op
   instead of recompiling ~1500 translation units when the build script re-runs
   (for example after a profile change).

Both the shim and VTK are static, so a built binary has no VTK shared library
dependency at all.  Delete `target/rvtk-vtk/` to force a rebuild of VTK.

#### Build times

Measured on an 18-core Apple Silicon machine, `cargo build -p rvtk --tests`:

| Scenario | Time |
| -------- | ---- |
| Cold (`cargo clean`): download + VTK + shim + Rust | **~11 min** |
| …of which VTK | ~8 min |
| …of which the shim | ~1 min 45 s |
| …of which Rust (crate + tests) | ~30 s |
| Warm, `--tests` | ~6 s |
| Warm, one VTK-independent test rebuilt | ~2 s |

The VTK build dominates and only happens once per VTK version, options and
machine.  Reusing it across checkouts or CI jobs is the point of the two cache
variables below.

#### Skipping the VTK build

When VTK has already been built somewhere (a CI cache, a tarball, another
checkout), point `RVTK_VTK_PREBUILT_DIR` at the install prefix and the download
and VTK build are skipped entirely:

```sh
# A prefix produced by a previous run lives at:
#   target/rvtk-vtk/vtk-9.7.1/build/
RVTK_VTK_PREBUILT_DIR=/path/to/vtk-9.7.1-install cargo build -p rvtk-sys
# -> ~1 min 30 s instead of ~11 min
```

The prefix is validated to be exactly VTK 9.7.1 (a mismatched tree is an error,
not a silent mismatch with the generated bindings).  To keep the static
guarantee the prefix must be a *static* VTK build (`BUILD_SHARED_LIBS=OFF`,
`CMAKE_POSITION_INDEPENDENT_CODE=ON`); a shared build also links, on platforms
where the dylibs are found at run time.  `RVTK_CACHE_DIR` relocates the
`vtk-<version>/{build,shim}` cache so several checkouts can share one copy.

### Why the smoke test is split into `section_*` functions

The generated `rvtk/tests/smoke.rs` calls every wrapped method (~12k checks).
Emitting those checks into a single `main` made rustc's type checking
super-linear: the front-end alone took minutes on one core.  `rvtk-gen` now
emits them into numbered `section_*` functions (called in order from `main`),
which brings a clean build of the test back to well under a minute and lets
codegen parallelise.  The `RVTK_SKIP` crash-hunting hook is unaffected.

| Variable | Effect |
| -------- | ------ |
| `RVTK_VTK_SOURCE_DIR` | Use this VTK source tree instead of downloading one (still built here, still static). |
| `RVTK_VTK_PREBUILT_DIR` | Use this already installed VTK and skip the download + build (must be 9.7.1; static to keep the static guarantee). |
| `RVTK_CACHE_DIR` | Put the VTK + shim cache here instead of `target/rvtk-vtk/`, so checkouts/CI jobs can share it. |
| `RVTK_VTK_URL` | Download from this URL instead of the pinned release. |
| `DOCS_RS` | Set by docs.rs; skips the native build entirely. |

## Design

### Reference counting

VTK objects are reference counted.  Each Rust wrapper owns one reference
(`New()` gives the first one):

* `Clone` calls `Register()` and duplicates the handle;
* `Drop` calls `Delete()`;
* `as_ptr()` exposes the raw pointer for advanced use.

Method results that return objects are wrapped with `from_borrowed`, which takes
an extra reference, so they are safe even when the callee keeps ownership.  A
side effect is that methods that *transfer* ownership (factory-style getters)
can leak one reference; correctness is never compromised, only reference counts
can stay above zero.

### Inheritance

Rust has no inheritance, so each wrapper implements `Deref` to its nearest
wrapped base class.  Because all wrappers are `#[repr(transparent)]` over the
same pointer type, upcasting is a pointer cast, and base class methods are
reachable directly through method resolution:

```rust
sphere.update();          // vtkAlgorithm::Update
sphere.get_output();      // vtkPolyDataAlgorithm::GetOutput
sphere.get_class_name();  // vtkObjectBase::GetClassName
```

### Overloads

C++ overloading is resolved by name mangling at generation time.  The overload
with the *fewest* parameters keeps the plain snake_case name, and the others get
a `_v2`, `_v3`, … suffix; ties are broken in favour of VTK's canonical scalar
types (`double` over `float`, scalars over fixed size arrays):

```rust
sphere.update();          // Update()
sphere.update_v2(0);      // Update(int port)
```

Every method also gets a `#[doc(alias = "CxxName")]`, so searching the generated
documentation by the original VTK name still finds it.

### Type mapping

| C++ | Rust |
| --- | ---- |
| `int`, `double`, … | `i32`, `f64`, … |
| `long` / `unsigned long` | `i64` / `u64` (through `c_long`/`c_ulong`) |
| `vtkIdType` | `i64` |
| `bool` | `bool` |
| `const char *` | `&str` (parameter), `String` (result) |
| `std::string` | `&str` (parameter), `String` (result) |
| `T *GetX()` where `T : vtkObjectBase` | `Option<T>` |
| `const double x[3]` | `[f64; 3]` |
| `double x[]` | `&[f64]` |

Anything that cannot be represented across a stable C ABI (multi-dimensional
arrays, raw out-parameters, function pointers, templates, `ostream`, …) is
skipped for that method; the generator reports how many methods it skipped.

## Regenerating the bindings

The generated sources are committed so that users do not need the wrapping tools.
To regenerate them against a different VTK version or module set:

```sh
./scripts/regenerate.sh
# or pick modules explicitly:
MODULES="vtkCommonCore;vtkCommonDataModel;vtkFiltersSources;vtkRenderingCore" \
  ./scripts/regenerate.sh
```

The script builds [WrapVTK](https://github.com/dgobbi/WrapVTK)'s `vtkWrapXML`
and invokes `rvtk-gen`:

```sh
cargo run -p rvtk-gen -- \
  --xml-dir /path/to/WrapVTK/build/xml \
  --repo . \
  --modules vtkCommonCore,vtkCommonDataModel,vtkFiltersSources
```

## Limitations / roadmap

* Only a subset of VTK modules is wrapped by default; add more with
  `MODULES=... ./scripts/regenerate.sh`.
* Value types (`vtkVector`, `vtkIndent`, `vtkVariant`, …) and `std::string`
  behind a pointer are not yet supported.
* Templated classes are skipped; `vtkSmartPointer<T>` signatures are supported
  by including the referenced header.
* Output parameters passed by reference (`T &`) and multi-dimensional arrays are
  skipped.
* Callback / observer support (`AddObserver`) is not wrapped yet.
* Some VTK APIs require the caller to set things up first.  For example
  `vtkPolyData::InsertNextCell` needs the target `vtkCellArray` to exist
  (`vtkPolyData::SetPolys`, as in VTK's own documentation); the binding is a
  faithful `unsafe`-free wrapper of the C++ contract, not a guard rail.
* A `pyvista`-style high level crate would be a good next step.

## Acknowledgements

* [VTK](https://vtk.org/) and its wrapping tools.
* [WrapVTK](https://github.com/dgobbi/WrapVTK) by David Gobbi, which produces the
  XML API description used here.
* The earlier [vtk-rs](https://github.com/jonaspleyer/vtk-rs) project and the
  [VTK forum discussion](https://discourse.vtk.org/t/rust-bindings/15700), which
  established the WrapVTK-based approach.
