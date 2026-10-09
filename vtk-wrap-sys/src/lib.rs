//! Low-level (unsafe) bindings to the Visualization Toolkit.
//!
//! The `ffi` module contains one `extern "C"` function per wrapped VTK method.
//! These functions are implemented by a small C++ shim that is compiled and
//! linked by [`build.rs`](../build.rs).  Every function takes the receiver as an
//! opaque `*mut c_void` and returns C-compatible values.
//!
//! Prefer the safe wrappers in the `vtk` crate.  This crate exists so that the
//! safe layer can be regenerated independently and so that users can drop down
//! to the raw ABI when needed.

#![allow(
    non_camel_case_types,
    non_snake_case,
    dead_code,
    improper_ctypes,
    clippy::all
)]

pub mod ffi {
    //! Raw C ABI types and functions.
    //!
    //! The thousands of generated declarations live in `generated.rs`.

    include!("generated.rs");

    extern "C" {
        /// Increment the reference count of a `vtkObjectBase`.
        pub fn vtk_wrap_register(obj: *mut ::core::ffi::c_void);
        /// Decrement the reference count of a `vtkObjectBase`, deleting it at 0.
        pub fn vtk_wrap_delete(obj: *mut ::core::ffi::c_void);
        /// Duplicate a C string on the heap.
        pub fn vtk_wrap_strdup(s: *const ::core::ffi::c_char) -> *mut ::core::ffi::c_char;
        /// Free a pointer returned by `vtk_wrap_strdup`.
        pub fn vtk_wrap_free(p: *mut ::core::ffi::c_void);
    }
}
