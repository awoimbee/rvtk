# Binding design

[Back to the README](../README.md)

The generated Rust API mirrors VTK's C++ API. A thin `extern "C"` shim
casts opaque pointers back to their native types and forwards each call;
`vtk-wrap-sys` declares those functions and `vtk-wrap` wraps them.

## Reference counting

Each Rust wrapper owns one VTK reference:

- Construction through `New()` supplies the initial reference.
- `Clone` calls `Register()` to retain another reference.
- `Drop` calls `Delete()` to release its reference.
- `as_ptr()` exposes the native pointer for advanced use.

Object results are wrapped with `from_borrowed`, which takes an extra
reference so the Rust handle can outlive the native owner's reference.
Methods that transfer an owned reference can therefore leak: the generator
does not currently distinguish all ownership-transfer conventions.

Rust wrappers do not validate all native preconditions. For example,
`vtkPolyData::InsertNextCell` requires an appropriate `vtkCellArray` to
have been set first with `vtkPolyData::SetPolys`. Violating a native API
contract can cause a crash even through a Rust method that is not marked
`unsafe`.

## Inheritance

Every wrapper is a `#[repr(transparent)]` newtype around a pointer.
`Deref` targets the nearest wrapped base class, making inherited methods
available through normal method resolution:

```rust
sphere.update();          // vtkAlgorithm::Update
sphere.get_output();      // vtkPolyDataAlgorithm::GetOutput
sphere.get_class_name();  // vtkObjectBase::GetClassName
```

## Overloads and naming

Methods use snake_case names. The overload with the fewest parameters keeps
the unsuffixed name; others receive `_v2`, `_v3`, and so on. Ties favor
VTK's canonical scalar types and scalar arguments over fixed-size arrays.

```rust
sphere.update();          // Update()
sphere.update_v2(0);      // Update(int port)
```

Each method has a `#[doc(alias = "CxxName")]` so rustdoc searches can use
the original C++ name.

## Type mapping

| C++ | Rust wrapper |
| --- | --- |
| `int`, `double`, … | `i32`, `f64`, … |
| `long` / `unsigned long` | `i64` / `u64` (FFI uses `c_long` / `c_ulong`) |
| `vtkIdType` | `i64` |
| `bool` | `bool` |
| `const char *`, `std::string` | `&str` parameters, `String` results |
| `T *GetX()` where `T : vtkObjectBase` | `Option<T>` |
| `const double x[3]` | `[f64; 3]` |
| `double x[3]` parameter | `&mut [f64; 3]` |
| `const double x[]` parameter | `&[f64]` |
| `double x[]` parameter | `&mut [f64]` |

Value types, most templates, reference output parameters, function pointers,
multi-dimensional arrays, and stream types are not wrapped.
`vtkSmartPointer<T>` signatures are supported where the generator can
resolve the referenced class. The generator reports skipped methods and
classes when it runs.

## Module-level generation

Generated bindings and smoke checks are split into one file per VTK module.
Feature-gated `include!` declarations keep disabled modules out of parsing
and compilation. The native build uses the same feature set to select VTK
libraries and shim translation units.

The generator implementation is in [`vtk-wrap-gen/src`](../vtk-wrap-gen/src).
For regeneration commands, see [Regenerating bindings](generating.md).
