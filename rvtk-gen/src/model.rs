//! Intermediate representation of the (subset of the) VTK API that we are able
//! to wrap.
//!
//! The representation is intentionally small: it only models what the code
//! generators need.  Types that cannot be mapped to a stable C ABI are rejected
//! while the IR is built, and the method that uses them is dropped (with a
//! warning).  This keeps the generated code compilable for the whole of VTK,
//! which has a very large and heterogeneous API surface.

use std::collections::BTreeMap;

/// The whole wrapped API.
#[derive(Debug, Default)]
pub struct Api {
    /// Refcounted classes (derived from `vtkObjectBase`), keyed by class name.
    pub classes: BTreeMap<String, Class>,
    /// VTK module names that were used, e.g. `vtkCommonCore`.
    pub modules: Vec<String>,
    /// Map from class name to the header that declares it.
    pub headers: BTreeMap<String, String>,
    /// Number of methods that were skipped because a type was unsupported.
    pub skipped_methods: usize,
    /// Number of classes that were skipped because VTK marks them deprecated.
    pub skipped_deprecated_classes: usize,
    /// Number of classes that were skipped because VTK only declares their
    /// interface for wrappers.
    pub skipped_wrapper_shims: usize,
}

#[derive(Debug, Clone)]
pub struct Class {
    pub name: String,
    pub module: String,
    pub header: String,
    pub is_abstract: bool,
    /// Direct public base class, if it is itself wrapped.
    pub base: Option<String>,
    /// Full inheritance chain (nearest first), used when the direct base is not
    /// wrapped.
    pub deref_target: Option<String>,
    /// `Some(expr)` when the class can be instantiated, where `expr` is a C++
    /// expression returning `T*` with one reference (e.g. `vtkSphereSource::New()`).
    pub construct_expr: Option<String>,
    pub methods: Vec<Method>,
    pub doc: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Method {
    /// Original C++ method name.
    pub cxx_name: String,
    /// Unique name of the generated `extern "C"` symbol.
    pub c_name: String,
    /// Name exposed in the safe Rust wrapper.
    pub rust_name: String,
    pub is_static: bool,
    pub params: Vec<Param>,
    /// `None` means the method returns `void`.
    pub ret: Option<Ty>,
    pub doc: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Param {
    pub name: String,
    pub ty: Ty,
    pub default: Option<String>,
}

/// A type that has been mapped onto a C ABI representation.
#[derive(Debug, Clone, PartialEq)]
pub enum Ty {
    // primitive scalars
    CChar,
    I8,
    U8,
    I16,
    U16,
    I32,
    U32,
    I64,
    U64,
    /// C `long` (platform dependent width, 32-bit on Windows).
    CLong,
    /// C `unsigned long`.
    CULong,
    Usize,
    Isize,
    F32,
    F64,
    Bool,
    /// `const char *`
    CStr,
    /// `char *` (mutable)
    CStrMut,
    /// `std::string`
    StdString,
    /// Opaque pointer to a refcounted VTK class.
    Object(String),
    /// A `vtkSmartPointer<T>` returned by value.
    SmartObject(String),
    /// Fixed size array, e.g. `double[3]`.  The `bool` is `true` when the
    /// array is `const` (an input).
    Array(Box<Ty>, usize, bool),
    /// Variable length array, e.g. `const double pts[]`.  Only valid as a
    /// parameter.
    Slice(Box<Ty>, bool),
    /// `void *`
    RawPtr,
}

impl Ty {
    /// The type used in the generated C++ `extern "C"` signature.
    pub fn c_type(&self) -> String {
        use Ty::*;
        match self {
            CChar => "char",
            I8 => "int8_t",
            U8 => "uint8_t",
            I16 => "int16_t",
            U16 => "uint16_t",
            I32 => "int32_t",
            U32 => "uint32_t",
            I64 => "int64_t",
            U64 => "uint64_t",
            CLong => "long",
            CULong => "unsigned long",
            Usize => "size_t",
            Isize => "ptrdiff_t",
            F32 => "float",
            F64 => "double",
            Bool => "bool",
            CStr => "const char*",
            CStrMut => "char*",
            // In a parameter position a `std::string` is received as a C string.
            StdString => "const char*",
            Object(_) | SmartObject(_) => "void*",
            Array(inner, _, is_const) => {
                if *is_const {
                    return format!("const {}*", inner.scalar_c());
                } else {
                    return format!("{}*", inner.scalar_c());
                }
            }
            Slice(inner, is_const) => {
                if *is_const {
                    return format!("const {}*", inner.scalar_c());
                } else {
                    return format!("{}*", inner.scalar_c());
                }
            }
            RawPtr => "void*",
        }
        .to_string()
    }

    /// The C++ type returned from the shim for this type (only differs for
    /// arrays and strings).
    pub fn c_return(&self) -> String {
        use Ty::*;
        match self {
            Array(inner, _, _) => format!("{}*", inner.scalar_c()),
            Slice(inner, _) => format!("{}*", inner.scalar_c()),
            Object(_) | SmartObject(_) => "void*".to_string(),
            // Returned strings are heap allocated and must be released with
            // `rvtk_free`.
            StdString => "char*".to_string(),
            other => other.c_type(),
        }
    }

    fn scalar_c(&self) -> String {
        use Ty::*;
        match self {
            CChar => "char",
            I8 => "int8_t",
            U8 => "uint8_t",
            I16 => "int16_t",
            U16 => "uint16_t",
            I32 => "int32_t",
            U32 => "uint32_t",
            I64 => "int64_t",
            U64 => "uint64_t",
            CLong => "long",
            CULong => "unsigned long",
            Usize => "size_t",
            Isize => "ptrdiff_t",
            F32 => "float",
            F64 => "double",
            Bool => "bool",
            other => panic!("not a scalar: {other:?}"),
        }
        .to_string()
    }

    /// The Rust type used in the `extern "C"` declaration.
    pub fn ffi_type(&self) -> String {
        use Ty::*;
        match self {
            CChar => "::core::ffi::c_char",
            I8 => "i8",
            U8 => "u8",
            I16 => "i16",
            U16 => "u16",
            I32 => "i32",
            U32 => "u32",
            I64 => "i64",
            U64 => "u64",
            CLong => "::core::ffi::c_long",
            CULong => "::core::ffi::c_ulong",
            Usize => "usize",
            Isize => "isize",
            F32 => "f32",
            F64 => "f64",
            Bool => "bool",
            CStr => "*const ::core::ffi::c_char",
            CStrMut => "*mut ::core::ffi::c_char",
            StdString => "*const ::core::ffi::c_char",
            Object(_) | SmartObject(_) => "*mut ::core::ffi::c_void",
            Array(inner, _, is_const) => {
                if *is_const {
                    return format!("*const {}", inner.ffi_scalar());
                } else {
                    return format!("*mut {}", inner.ffi_scalar());
                }
            }
            Slice(inner, is_const) => {
                if *is_const {
                    return format!("*const {}", inner.ffi_scalar());
                } else {
                    return format!("*mut {}", inner.ffi_scalar());
                }
            }
            RawPtr => "*mut ::core::ffi::c_void",
        }
        .to_string()
    }

    /// The Rust type returned from the `extern "C"` declaration.
    pub fn ffi_return(&self) -> String {
        use Ty::*;
        match self {
            Array(inner, _, _) => format!("*mut {}", inner.ffi_scalar()),
            Slice(inner, _) => format!("*mut {}", inner.ffi_scalar()),
            StdString => "*mut ::core::ffi::c_char".to_string(),
            other => other.ffi_type(),
        }
    }

    fn ffi_scalar(&self) -> String {
        self.ffi_type()
    }

    /// The public Rust type of a parameter.
    pub fn rust_param(&self) -> String {
        use Ty::*;
        match self {
            CChar => "::core::ffi::c_char".into(),
            I8 => "i8".into(),
            U8 => "u8".into(),
            I16 => "i16".into(),
            U16 => "u16".into(),
            I32 => "i32".into(),
            U32 => "u32".into(),
            I64 => "i64".into(),
            U64 => "u64".into(),
            CLong => "i64".into(),
            CULong => "u64".into(),
            Usize => "usize".into(),
            Isize => "isize".into(),
            F32 => "f32".into(),
            F64 => "f64".into(),
            Bool => "bool".into(),
            CStr | CStrMut | StdString => "&str".into(),
            Object(name) | SmartObject(name) => format!("&{name}"),
            Array(inner, n, is_const) => {
                if *is_const {
                    // fixed size inputs are `Copy`, take them by value
                    format!("[{}; {}]", inner.rust_param(), n)
                } else {
                    format!("&mut [{}; {}]", inner.rust_param(), n)
                }
            }
            Slice(inner, is_const) => {
                if *is_const {
                    format!("&[{}]", inner.rust_param())
                } else {
                    format!("&mut [{}]", inner.rust_param())
                }
            }
            RawPtr => "*mut ::core::ffi::c_void".into(),
        }
    }

    /// The public Rust return type.
    pub fn rust_return(&self) -> String {
        use Ty::*;
        match self {
            CStr | CStrMut | StdString => "String".into(),
            Object(name) | SmartObject(name) => format!("Option<{name}>"),
            Array(inner, n, _) => format!("[{}; {}]", inner.rust_param(), n),
            Slice(inner, _) => format!("&[{}]", inner.rust_param()),
            RawPtr => "*mut ::core::ffi::c_void".into(),
            other => other.rust_param(),
        }
    }

    /// True when the type is a plain scalar that needs no conversion.
    pub fn is_scalar(&self) -> bool {
        use Ty::*;
        matches!(
            self,
            CChar
                | I8
                | U8
                | I16
                | U16
                | I32
                | U32
                | I64
                | U64
                | CLong
                | CULong
                | Usize
                | Isize
                | F32
                | F64
                | Bool
                | RawPtr
        )
    }

    pub fn object_name(&self) -> Option<&str> {
        match self {
            Ty::Object(name) | Ty::SmartObject(name) => Some(name),
            _ => None,
        }
    }
}
