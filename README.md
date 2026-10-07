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
* An installed VTK (≥ 9.1) with its CMake package files, e.g.
  * macOS: `brew install vtk`
  * Debian/Ubuntu: `apt install libvtk9-dev`
  * Arch: `pacman -S vtk`
* `VTK_DIR` may be set to the directory containing `vtk-config.cmake`.  If it is
  not, the build script looks in Homebrew and the usual system prefixes.

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
per-class XML ──► rvtk-gen ──┬─► rvtk-sys/shim/**.cpp  ──► librvtk_shim
                             ├─► rvtk-sys/src/generated.rs
                             └─► rvtk/src/generated.rs
```

## Building

```sh
cargo build -p rvtk-sys -p rvtk
cargo test  -p rvtk
cargo run   -p rvtk --example sphere_source
```

`rvtk-sys/build.rs` finds VTK, configures `rvtk-sys/shim` with CMake and links the
resulting `librvtk_shim`.  The shim is a thin `extern "C"` layer: every function
casts the opaque receiver back to its concrete type and performs the call, so
the code the C++ compiler sees is essentially identical to hand written code.

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
