//! Safe (well, mostly safe) Rust bindings to the Visualization Toolkit (VTK).
//!
//! This crate is generated from VTK's own wrapping metadata.  Every wrapped
//! class becomes a `#[repr(transparent)]` newtype around a reference counted
//! VTK pointer:
//!
//! * construction goes through the class' `New()` factory;
//! * `Clone` takes another reference, `Drop` releases it;
//! * inheritance is modelled with `Deref`, so methods declared in base classes
//!   are reachable directly (for example `vtkSphereSource::modified()`).
//!
//! ```no_run
//! use vtk::vtkSphereSource;
//!
//! let sphere = vtkSphereSource::new();
//! sphere.set_radius(2.0);
//! assert_eq!(sphere.get_radius(), 2.0);
//! ```
//!
//! The bindings are deliberately low level: they mirror VTK's API rather than
//! hiding it.  Higher level, `pyvista`-style helpers belong in a separate crate.

#![allow(
    non_camel_case_types,
    non_snake_case,
    dead_code,
    unused_imports,
    unused_unsafe,
    clippy::all
)]

mod generated;

pub use generated::*;

/// Commonly used re-exports.
pub mod prelude {
    pub use crate::*;
}
