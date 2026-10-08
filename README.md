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
  `IOPLY` and `ImagingHybrid` — **1582 classes and ~35 600 methods**.
* The module list is configurable, and a VTK module is only built if its Cargo
  feature is enabled; see [Feature flags](#feature-flags) and
  [Regenerating the bindings](#regenerating-the-bindings).

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
* Network access the first time (to download the pinned VTK source release), or a
  VTK source tree pointed at by `RVTK_VTK_SOURCE_DIR`.
* No installed VTK is used: `rvtk-sys` builds the pinned VTK release from source
  and links it statically (see [Building](#building)).

## Workspace layout

| Crate | Lib name | Purpose |
| ----- | -------- | ------- |
| `rvtk` | `vtk` | Safe wrappers: one `#[repr(transparent)]` newtype per class, reference counting, inheritance via `Deref`. |
| `rvtk-sys` | `rvtk_sys` | Raw `extern "C"` declarations (`rvtk_sys::ffi`) and the CMake project that builds the C++ shim. |
| `rvtk-gen` | — | The code generator.  Clones/builds WrapVTK, generates the XML API description, and emits the C++ shim, the FFI crate and the wrapper crate. |

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

### Feature flags

VTK is a collection of ~160 static libraries, and building all of them is most
of the build time.  Every wrapped module is therefore a Cargo feature named
after it, and **only the enabled modules are compiled**: `rvtk-sys/build.rs`
passes the enabled list to CMake, which builds just those VTK libraries and
just those shim translation units.

The default is deliberately small:

| Default features | Pulls in |
| ---------------- | -------- |
| `vtkCommonCore` | the object model |
| `vtkCommonDataModel` | datasets |
| `vtkFiltersCore` | the core pipeline filters |

Enabling a feature also enables the modules it depends on, so the closure of
the three defaults is `vtkCommonCore`, `vtkCommonDataModel`,
`vtkCommonExecutionModel`, `vtkCommonMath`, `vtkCommonMisc`, `vtkCommonSystem`,
`vtkCommonTransforms` and `vtkFiltersCore` — eight VTK libraries instead of the
~160 a full VTK builds.  Ask for more by name:

```sh
cargo build -p rvtk --features vtkFiltersSources,vtkRenderingCore
```

The features select both the VTK libraries that are built *and* the generated
bindings that exist.  The generated code is split into one file per module
(`rvtk/src/generated/<module>.rs`, and likewise for `rvtk-sys` and the smoke
test); each file is `include!`d only when its feature is on, so a disabled
module is never even parsed.

> A `#[cfg(feature = "…")]` on every generated item would be far simpler, but
> rustc's cost is super-linear in the number of gated items: ~9300 attributes
> pushed a `cargo check -p rvtk` from ~2 s past 150 s.  File-level gating keeps
> it at ~2 s.

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

### Platform support and CI

`.github/workflows/ci.yml` builds the whole workspace (native VTK + shim + Rust)
and runs the fast tests on Linux, macOS and Windows for every pull request.

The committed bindings were generated from a **macOS** VTK, so today only the
macOS job can pass:

* they wrap classes that only exist there (the generated shim `#include`s
  `vtkCocoaRenderWindow.h`, `vtkCocoaHardwareWindow.h` and
  `vtkCocoaRenderWindowInteractor.h`, which no other platform installs);
* the generated C++ spells 64-bit ids `int64_t`, which is the same type as
  `vtkIdType` on macOS (`long long`) but not on Linux/Windows (`long` vs
  `long long`), so a handful of array parameters fail to compile;
* on Windows the default Visual Studio generator writes no `link.txt`, which
  `build.rs` reads to reconstruct the link line.

Because of that the Linux and Windows jobs are marked `continue-on-error`:
they run the same steps and report their status without blocking a PR.  Making
them blocking means generating the bindings per platform — a per-platform
module list (to drop the Cocoa classes) and a type-faithful mapping between
`vtkIdType`/`vtkTypeInt64` and the C ABI — which is the next step if
cross-platform support is wanted.

### Pre-commit hooks

`.pre-commit-config.yaml` runs three checks, via [prek](https://github.com/j178/prek)
(a Rust implementation of pre-commit that reads the same file):

| Hook | Checks |
| ---- | ------ |
| `cargo fmt` | Formatting, across the workspace. |
| [`typos`](https://github.com/crate-ci/typos) | Spelling, in identifiers and comments. |
| [`zizmor`](https://github.com/zizmorcore/zizmor) | The GitHub Actions workflow. |

```sh
prek install          # once, to wire up the git hook
prek run --all-files  # everything, over the whole tree
prek run              # just the staged files
```

The same three hooks run in CI as the `lint` job (`.github/workflows/ci.yml`),
which needs no VTK and finishes in well under a minute, so it gates the
multi-minute native build rather than running alongside it.  `cargo fmt` is a
check there, not a fix: a pre-commit hook that reformats a file counts as a
failure, so an unformatted PR is rejected.

Generated code is deliberately out of scope for formatting and spelling: it is
rewritten from scratch by `rvtk-gen` on regeneration, so touching it would only
create churn.  `.typos.toml` excludes those paths, and the `cargo fmt` hook
formats from the crate roots rather than passing filenames — rustfmt follows
`mod x;` but not `include!`, and every generated file is reached through
`include!`, so a workspace-wide `cargo fmt` leaves all ~470k generated lines
alone.

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
`rvtk-gen` owns the whole pipeline: it clones (or reuses) [WrapVTK], builds its
`vtkWrapXML` tool against VTK, generates the XML API description, and writes the
C++ shim and the Rust crates.

```sh
./scripts/regenerate.sh
# or pick modules explicitly (comma separated, VTK spelling):
MODULES=vtkCommonCore,vtkFiltersSources,vtkRenderingCore ./scripts/regenerate.sh
```

Equivalently, without the wrapper script:

```sh
cargo run -p rvtk-gen -- --repo .
```

### It uses the pinned VTK, not the system one

The bindings are generated from a *pinned* VTK release: the same 9.7.1 that
`rvtk-sys` downloads and builds (`VTK_VERSION` in `rvtk-sys/build.rs`).  On a
cold cache, build that once first:

```sh
cargo build -p rvtk-sys     # downloads and builds the pinned VTK
```

`rvtk-gen` then finds it under `target/rvtk-vtk/vtk-<version>/build` (or
`$RVTK_CACHE_DIR`), so regeneration does not depend on any VTK being installed.
The lookup order is `--vtk-dir`, `VTK_DIR`, the pinned build, then a system
install.  A VTK without the `WrappingTools` component cannot be used; the pinned
build always has it.

### The module set comes from the features

The modules to wrap default to the `vtk*` Cargo features declared in
`rvtk-sys/Cargo.toml`, so the generated XML, the bindings and the features
cannot drift apart.  `--modules` (or `MODULES=...`) overrides that; passing
nothing when the manifest has no `vtk*` features wraps everything VTK exposes.

| `rvtk-gen` option | Purpose |
| ----------------- | ------- |
| `--xml-dir` | Use pre-generated XML and skip WrapVTK entirely. |
| `--vtk-dir` | VTK CMake package dir to build `vtkWrapXML` against. |
| `--vtk-include` | VTK include dir, to detect wrapper-only array shims. |
| `--wrapvtk-dir` | WrapVTK checkout to use or create (default `target/wrapvtk`). |
| `--wrapvtk-url` | Git URL to clone WrapVTK from. |
| `--modules` | Modules to wrap (default: the `rvtk-sys` features). |
| `--jobs` | Passed to `cmake --build --parallel` for WrapVTK. |

[WrapVTK]: https://github.com/dgobbi/WrapVTK

## Limitations / roadmap

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

## Related projects

* [vtk-pure-rs](https://github.com/henriksson-lab/vtk-pure-rs) takes the
  opposite approach to this crate.  Instead of binding VTK it is a ground-up
  Rust reimplementation of VTK 9.6 (an LLM-mediated translation of the C++
  source), with no C++ toolchain and no system VTK involved.  It is
  experimental and, by its own benchmarks, currently slower than the C++
  original; in exchange it builds anywhere Cargo does, including to
  WebAssembly, which is exactly what an FFI binding like this one cannot do.

## Acknowledgements

* [VTK](https://vtk.org/) and its wrapping tools.
* [WrapVTK](https://github.com/dgobbi/WrapVTK) by David Gobbi, which produces the
  XML API description used here.
* The earlier [vtk-rs](https://github.com/jonaspleyer/vtk-rs) project and the
  [VTK forum discussion](https://discourse.vtk.org/t/rust-bindings/15700), which
  established the WrapVTK-based approach.
