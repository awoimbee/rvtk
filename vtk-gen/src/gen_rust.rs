//! Rust binding generation: the raw `extern "C"` declarations for `vtk-sys` and
//! the safe wrappers for the `vtk` crate.

use std::fmt::Write as _;

use crate::gen_cpp::ctor_c_name;
use crate::model::{Api, Method, Param, Ty};

fn doc_block(out: &mut String, indent: &str, doc: Option<&str>, alias: Option<&str>) {
    if let Some(alias) = alias {
        let _ = writeln!(out, "{indent}#[doc(alias = \"{alias}\")]");
    }
    if let Some(doc) = doc {
        for line in doc.lines() {
            let line = line.replace('\\', "\\\\");
            let _ = writeln!(out, "{indent}///{line}");
        }
    }
}

fn ffi_signature(m: &Method) -> String {
    let mut params = Vec::new();
    if !m.is_static {
        params.push("self_: *mut ::core::ffi::c_void".to_string());
    }
    for p in &m.params {
        params.push(format!("{}: {}", p.name, p.ty.ffi_type()));
    }
    let ret = m
        .ret
        .as_ref()
        .map(|t| t.ffi_return())
        .unwrap_or_else(|| "()".into());
    format!("pub fn {}({}) -> {ret};", m.c_name, params.join(", "))
}

/// Render the `let` statements and argument expressions used to call a method.
fn prepare_args(m: &Method) -> (String, Vec<String>) {
    let mut prelude = String::new();
    let mut args = Vec::new();
    if !m.is_static {
        args.push("self.as_ptr()".to_string());
    }
    for (i, Param { name, ty, .. }) in m.params.iter().enumerate() {
        match ty {
            Ty::CLong => args.push(format!("{name} as ::core::ffi::c_long")),
            Ty::CULong => args.push(format!("{name} as ::core::ffi::c_ulong")),
            Ty::CStr | Ty::CStrMut | Ty::StdString => {
                let _ = writeln!(
                    prelude,
                    "    let __s{i} = ::std::ffi::CString::new({name}).expect(\"string contains a NUL byte\");"
                );
                if matches!(ty, Ty::CStrMut) {
                    args.push(format!("__s{i}.as_ptr() as *mut ::core::ffi::c_char"));
                } else {
                    args.push(format!("__s{i}.as_ptr()"));
                }
            }
            Ty::Object(_) | Ty::SmartObject(_) => args.push(format!("{name}.as_ptr()")),
            Ty::Array(_, _, is_const) | Ty::Slice(_, is_const) => {
                if *is_const {
                    args.push(format!("{name}.as_ptr()"));
                } else {
                    args.push(format!("{name}.as_mut_ptr()"));
                }
            }
            Ty::F32 => args.push(format!("{name}")),
            Ty::F64 => args.push(format!("{name}")),
            Ty::Bool => args.push(format!("{name}")),
            _ => args.push(name.clone()),
        }
    }
    (prelude, args)
}

fn zero_literal(ty: &Ty) -> &'static str {
    match ty {
        Ty::F32 => "0.0f32",
        Ty::F64 => "0.0f64",
        Ty::Bool => "false",
        _ => "0",
    }
}

fn render_body(m: &Method) -> String {
    let (prelude, args) = prepare_args(m);
    let call = format!(
        "unsafe {{ vtk_sys::ffi::{}({}) }}",
        m.c_name,
        args.join(", ")
    );
    let mut body = prelude;
    match &m.ret {
        None => {
            let _ = writeln!(body, "    {call};");
        }
        Some(Ty::CStr) => {
            let _ = writeln!(body, "    let __ret = {call};");
            let _ = writeln!(
                body,
                "    if __ret.is_null() {{ return String::new(); }}\n    unsafe {{ ::core::ffi::CStr::from_ptr(__ret) }}.to_string_lossy().into_owned()"
            );
        }
        Some(Ty::StdString) => {
            let _ = writeln!(body, "    let __ret = {call};");
            let _ = writeln!(
                body,
                "    if __ret.is_null() {{ return String::new(); }}\n    let __s = unsafe {{ ::core::ffi::CStr::from_ptr(__ret) }}.to_string_lossy().into_owned();\n    unsafe {{ vtk_sys::ffi::rvtk_free(__ret as *mut ::core::ffi::c_void) }};\n    __s"
            );
        }
        Some(Ty::Object(name)) | Some(Ty::SmartObject(name)) => {
            let _ = writeln!(body, "    unsafe {{ {name}::from_borrowed({call}) }}");
        }
        Some(Ty::Array(inner, n, _)) => {
            let zero = zero_literal(inner);
            let _ = writeln!(body, "    let __ret = {call};");
            let _ = writeln!(
                body,
                "    if __ret.is_null() {{ return [{zero}; {n}]; }}\n    let mut __out = [{zero}; {n}];\n    unsafe {{ ::core::ptr::copy_nonoverlapping(__ret, __out.as_mut_ptr(), {n}) }};\n    __out"
            );
        }
        Some(Ty::CLong) => {
            let _ = writeln!(body, "    ({call}) as i64");
        }
        Some(Ty::CULong) => {
            let _ = writeln!(body, "    ({call}) as u64");
        }
        Some(_) => {
            let _ = writeln!(body, "    {call}");
        }
    }
    body
}

fn emit_method(out: &mut String, m: &Method) {
    doc_block(out, "    ", m.doc.as_deref(), Some(&m.cxx_name));
    let mut params = Vec::new();
    if !m.is_static {
        params.push("&self".to_string());
    }
    for p in &m.params {
        params.push(format!("{}: {}", p.name, p.ty.rust_param()));
    }
    let ret = m
        .ret
        .as_ref()
        .map(|t| format!(" -> {}", t.rust_return()))
        .unwrap_or_default();
    let _ = writeln!(
        out,
        "    pub fn {}({}){ret} {{",
        m.rust_name,
        params.join(", ")
    );
    out.push_str(&render_body(m));
    let _ = writeln!(out, "    }}\n");
}

pub fn emit_ffi(api: &Api) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "// Generated by vtk-gen. Do not edit.\n");
    let _ = writeln!(out, "extern \"C\" {{");
    for class in api.classes.values() {
        if class.construct_expr.is_some() {
            let _ = writeln!(
                out,
                "    pub fn {}() -> *mut ::core::ffi::c_void;",
                ctor_c_name(&class.name)
            );
        }
        for m in &class.methods {
            let _ = writeln!(out, "    {}", ffi_signature(m));
        }
    }
    let _ = writeln!(out, "}}\n");
    out
}

pub fn emit_wrappers(api: &Api) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "// Generated by vtk-gen. Do not edit.");
    let _ = writeln!(out, "use ::core::ffi::c_void;");
    let _ = writeln!(out, "use ::core::ptr::NonNull;\n");

    for class in api.classes.values() {
        doc_block(&mut out, "", class.doc.as_deref(), None);
        let _ = writeln!(out, "#[repr(transparent)]");
        let _ = writeln!(out, "pub struct {}(NonNull<c_void>);\n", class.name);

        let _ = writeln!(out, "impl {} {{", class.name);
        let _ = writeln!(out, "    /// Raw VTK pointer, borrowed from this handle.");
        let _ = writeln!(
            out,
            "    #[inline]\n    pub fn as_ptr(&self) -> *mut c_void {{ self.0.as_ptr() }}\n"
        );
        let _ = writeln!(
            out,
            "    /// Wrap a pointer that is already owned (reference count +1).\n    ///\n    /// # Safety\n    /// `ptr` must be a valid `{} *` and the caller transfers one reference.\n    #[inline]\n    pub(crate) unsafe fn from_owned(ptr: *mut c_void) -> Option<Self> {{\n        NonNull::new(ptr).map(Self)\n    }}",
            class.name
        );
        let _ = writeln!(
            out,
            "\n    /// Wrap a borrowed pointer, taking an extra reference.\n    ///\n    /// # Safety\n    /// `ptr` must be null or a valid `{} *`.\n    #[inline]\n    pub(crate) unsafe fn from_borrowed(ptr: *mut c_void) -> Option<Self> {{\n        let p = NonNull::new(ptr)?;\n        vtk_sys::ffi::rvtk_register(p.as_ptr());\n        Some(Self(p))\n    }}",
            class.name
        );

        if class.construct_expr.is_some() {
            let _ = writeln!(
                out,
                "\n    /// Construct a new `{}`.\n    pub fn new() -> Self {{\n        // SAFETY: the VTK factory returns an owned reference.\n        let __ptr = unsafe {{ vtk_sys::ffi::{}() }};\n        unsafe {{ Self::from_owned(__ptr) }}.expect(\"{}::New returned null\")\n    }}",
                class.name,
                ctor_c_name(&class.name),
                class.name
            );
        }
        out.push('\n');
        for m in &class.methods {
            emit_method(&mut out, m);
        }
        let _ = writeln!(out, "}}\n");

        // Reference counting.
        let _ = writeln!(
            out,
            "impl ::core::ops::Drop for {} {{\n    fn drop(&mut self) {{\n        // SAFETY: `self.0` holds a reference.\n        unsafe {{ vtk_sys::ffi::rvtk_delete(self.0.as_ptr()) }};\n    }}\n}}",
            class.name
        );
        let _ = writeln!(
            out,
            "impl ::core::clone::Clone for {} {{\n    fn clone(&self) -> Self {{\n        // SAFETY: registering is always safe for a live object.\n        unsafe {{ vtk_sys::ffi::rvtk_register(self.0.as_ptr()) }};\n        Self(self.0)\n    }}\n}}",
            class.name
        );
        if class.construct_expr.is_some() {
            let _ = writeln!(
                out,
                "impl ::core::default::Default for {} {{\n    #[inline]\n    fn default() -> Self {{ Self::new() }}\n}}",
                class.name
            );
        }

        // Upcast.
        if let Some(base) = &class.deref_target {
            let _ = writeln!(
                out,
                "impl ::core::ops::Deref for {} {{\n    type Target = {base};\n    #[inline]\n    fn deref(&self) -> &Self::Target {{\n        // SAFETY: all wrappers are `#[repr(transparent)]` over the same pointer.\n        unsafe {{ &*(self as *const Self as *const Self::Target) }}\n    }}\n}}",
                class.name
            );
        }

        let _ = writeln!(out, "impl ::core::fmt::Debug for {} {{\n    fn fmt(&self, f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {{\n        f.debug_tuple(\"{}\").field(&self.as_ptr()).finish()\n    }}\n}}", class.name, class.name);
    }
    out
}

/// A small summary of what was generated, printed by the CLI.
pub fn summary(api: &Api) -> String {
    let methods: usize = api.classes.values().map(|c| c.methods.len()).sum();
    let constructable = api
        .classes
        .values()
        .filter(|c| c.construct_expr.is_some())
        .count();
    format!(
        "modules: {}\nclasses: {}\nconstructable: {}\nmethods: {}\nskipped methods (unsupported types): {}",
        api.modules.len(),
        api.classes.len(),
        constructable,
        methods,
        api.skipped_methods
    )
}
