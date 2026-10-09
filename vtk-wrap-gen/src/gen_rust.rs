//! Rust binding generation: the raw `extern "C"` declarations for `vtk-wrap-sys` and
//! the safe wrappers for the `vtk` crate.

use std::fmt::Write as _;

use crate::gen_cpp::ctor_c_name;
use crate::model::{Api, Class, Method, Param, Ty};

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
        "unsafe {{ vtk_wrap_sys::ffi::{}({}) }}",
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
                "    if __ret.is_null() {{ return String::new(); }}\n    let __s = unsafe {{ ::core::ffi::CStr::from_ptr(__ret) }}.to_string_lossy().into_owned();\n    unsafe {{ vtk_wrap_sys::ffi::vtk_wrap_free(__ret as *mut ::core::ffi::c_void) }};\n    __s"
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

/// Raw `extern "C"` declarations for one VTK module.
///
/// One file per module and no `#[cfg]` inside: the file is only `include!`d
/// when its Cargo feature is on (see [`emit_ffi_index`]).  Putting a `#[cfg]`
/// on every one of the ~35k declarations instead costs minutes of compile
/// time, because rustc's cost is super-linear in the number of gated items.
pub fn emit_ffi_module(api: &Api, module: &str) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "// Generated by vtk-wrap-gen. Do not edit.");
    let _ = writeln!(
        out,
        "// Raw FFI declarations for the `{module}` VTK module.\n"
    );
    let _ = writeln!(out, "extern \"C\" {{");
    for class in api.classes.values().filter(|class| class.module == module) {
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

/// The include list that pulls in the enabled modules' declarations.
pub fn emit_ffi_index(modules: &[String]) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "// Generated by vtk-wrap-gen. Do not edit.");
    let _ = writeln!(out, "//");
    let _ = writeln!(
        out,
        "// One `include!` per VTK module, each behind its Cargo feature."
    );
    let _ = writeln!(out, "//");
    let _ = writeln!(
        out,
        "// A disabled module's file is never parsed, and the declarations"
    );
    let _ = writeln!(
        out,
        "// still land in the same `ffi` module, so cross-module types resolve."
    );
    // Plain comments: this file is `include!`d (not a module root), so `//!`
    // would have nothing to attach to.
    let _ = writeln!(out);
    for module in modules {
        let _ = writeln!(out, "#[cfg(feature = \"{module}\")]");
        let _ = writeln!(out, "include!(\"generated/{module}.rs\");");
    }
    out
}

/// Safe wrappers for one VTK module (struct, inherent impls, reference
/// counting and `Deref`), with no `#[cfg]` inside.
pub fn emit_wrappers_module(api: &Api, module: &str) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "// Generated by vtk-wrap-gen. Do not edit.");
    let _ = writeln!(out, "// Safe wrappers for the `{module}` VTK module.\n");

    for class in api.classes.values().filter(|class| class.module == module) {
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
            "\n    /// Wrap a borrowed pointer, taking an extra reference.\n    ///\n    /// # Safety\n    /// `ptr` must be null or a valid `{} *`.\n    #[inline]\n    pub(crate) unsafe fn from_borrowed(ptr: *mut c_void) -> Option<Self> {{\n        let p = NonNull::new(ptr)?;\n        vtk_wrap_sys::ffi::vtk_wrap_register(p.as_ptr());\n        Some(Self(p))\n    }}",
            class.name
        );

        if class.construct_expr.is_some() {
            let _ = writeln!(
                out,
                "\n    /// Construct a new `{name}`.\n    ///\n    /// # Panics\n    /// Panics if VTK's object factory cannot instantiate the class, which\n    /// happens for abstract classes.  Use [`Self::try_new`] to handle that\n    /// case instead of panicking.\n    pub fn new() -> Self {{\n        Self::try_new().expect(\"{name}::New returned null (abstract class?)\")\n    }}\n\n    /// Construct a new `{name}`, or `None` if VTK's object factory cannot\n    /// instantiate the class (abstract classes).\n    pub fn try_new() -> Option<Self> {{\n        // SAFETY: the VTK factory returns an owned reference, or null.\n        let __ptr = unsafe {{ vtk_wrap_sys::ffi::{ctor}() }};\n        unsafe {{ Self::from_owned(__ptr) }}\n    }}",
                name = class.name,
                ctor = ctor_c_name(&class.name),
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
            "impl ::core::ops::Drop for {} {{\n    fn drop(&mut self) {{\n        // SAFETY: `self.0` holds a reference.\n        unsafe {{ vtk_wrap_sys::ffi::vtk_wrap_delete(self.0.as_ptr()) }};\n    }}\n}}",
            class.name
        );
        let _ = writeln!(
            out,
            "impl ::core::clone::Clone for {} {{\n    fn clone(&self) -> Self {{\n        // SAFETY: registering is always safe for a live object.\n        unsafe {{ vtk_wrap_sys::ffi::vtk_wrap_register(self.0.as_ptr()) }};\n        Self(self.0)\n    }}\n}}",
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

/// The include list for the safe wrappers, mirroring [`emit_ffi_index`].
pub fn emit_wrappers_index(modules: &[String]) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "// Generated by vtk-wrap-gen. Do not edit.");
    let _ = writeln!(out, "//!");
    let _ = writeln!(
        out,
        "//! One `include!` per VTK module, each behind its Cargo feature."
    );
    let _ = writeln!(out, "//!");
    let _ = writeln!(
        out,
        "//! Textual inclusion keeps every wrapper in this one module, so the"
    );
    let _ = writeln!(
        out,
        "//! generated code can refer to base classes and argument types by their"
    );
    let _ = writeln!(
        out,
        "//! bare names.  A disabled module's file is never parsed.\n"
    );
    let _ = writeln!(out, "use ::core::ffi::c_void;");
    let _ = writeln!(out, "use ::core::ptr::NonNull;\n");
    for module in modules {
        let _ = writeln!(out, "#[cfg(feature = \"{module}\")]");
        let _ = writeln!(out, "include!(\"generated/{module}.rs\");");
    }
    out
}

/// Emit a generated crash-safety smoke test.
///
/// The test constructs every constructable class and checks the wrapper
/// invariants: the object really is an instance of the class it was created
/// from, it derives from `vtkObjectBase` (so the `Register`/`Delete` pair the
/// wrapper uses is valid), and a clone/drop round trip restores the reference
/// count.  It then checks `is_type_of` for every wrapped class, including the
/// ones that cannot be instantiated.
///
/// This is the broadest safety net we have: it touches every generated wrapper
/// type, every `New()` factory and the whole reference-counting path.  A crash
/// here means the safe API can abort the process, and the last line printed
/// names the offending class.
pub fn emit_smoke_module(api: &Api, module: &str) -> String {
    fn has(class: &Class, rust_name: &str) -> bool {
        class.methods.iter().any(|m| m.rust_name == rust_name)
    }

    // One `String` per top-level `check!(...)` statement, in the order they
    // must run.  They are emitted into several functions rather than one giant
    // `run`: a function holding tens of thousands of checks makes rustc's type
    // checking super-linear (minutes instead of seconds) and leaves codegen
    // nothing to parallelise.
    let mut statements: Vec<String> = Vec::new();

    // Static type information: valid for every wrapped class, constructable or
    // not.  `is_type_of("vtkObjectBase")` also proves the class is reference
    // counted, which is what makes `Register`/`Delete` sound.
    for class in api.classes.values().filter(|class| class.module == module) {
        let n = &class.name;
        if !has(class, "is_type_of") {
            continue;
        }
        statements.push(format!(
            "    check!(\"{n}::is_type_of\", || {{ assert_eq!({n}::is_type_of(\"vtkObjectBase\"), 1); }});\n"
        ));
    }

    // Construction, upcast and reference counting.
    for class in api.classes.values().filter(|class| class.module == module) {
        // Only `is_a` must be declared on the class itself; `get_reference_count`
        // and `get_class_name` are inherited from `vtkObjectBase` and reached
        // through `Deref` for every refcounted class.
        if class.construct_expr.is_none() || !has(class, "is_a") {
            continue;
        }
        let n = &class.name;
        let mut s = String::new();
        let _ = write!(s, "    check!(\"{n}::new\", || {{\n");
        let _ = write!(s, "        let Some(a) = {n}::try_new() else {{\n");
        let _ = write!(
            s,
            "            println!(\"      ({{}} cannot be instantiated: VTK factory returned null)\", \"{n}\");\n"
        );
        s.push_str("            return;\n        };\n");
        s.push_str("        assert_eq!(a.is_a(\"vtkObjectBase\"), 1, \"not refcounted\");\n");
        // VTK's object factory returns a concrete subclass (e.g.
        // `vtkPolyDataMapper::New()` gives a `vtkOpenGLPolyDataMapper`), and
        // template specialisations get a length-prefixed class name (e.g.
        // "18vtkAffineCharArray").  Either way the object must still *be* an
        // instance of the class the wrapper was created from.
        let _ = write!(
            s,
            "        assert!(a.is_a(\"{n}\") == 1 || a.get_class_name().ends_with(\"{n}\"), \"class name {{:?}} does not match the wrapper type\", a.get_class_name());\n"
        );
        s.push_str("        let before = a.get_reference_count();\n");
        s.push_str("        let b = a.clone();\n");
        s.push_str("        assert!(b.get_reference_count() > before, \"clone did not add a reference\");\n");
        s.push_str("        drop(b);\n");
        s.push_str("        assert_eq!(a.get_reference_count(), before, \"drop did not release exactly one reference\");\n");
        s.push_str("    });\n");
        statements.push(s);
    }

    // Read-only accessor sweep: call every `get_*`/`is_*`/`has_*` method that
    // takes no arguments on a live instance of each constructable class.  This
    // is what actually exercises the generated call glue, and a crash names the
    // exact class and method.
    for class in api.classes.values().filter(|class| class.module == module) {
        if class.construct_expr.is_none() || !has(class, "is_a") {
            continue;
        }
        let n = &class.name;
        let accessors: Vec<&Method> = class
            .methods
            .iter()
            .filter(|m| {
                !m.is_static
                    && m.params.is_empty()
                    && (m.rust_name.starts_with("get_")
                        || m.rust_name.starts_with("is_")
                        || m.rust_name.starts_with("has_"))
            })
            .collect();
        if accessors.is_empty() {
            continue;
        }
        let mut s = String::new();
        let _ = write!(s, "    check!(\"{n}\", || {{\n");
        let _ = write!(
            s,
            "        let Some(a) = {n}::try_new() else {{ return; }};\n"
        );
        for m in accessors {
            let rn = &m.rust_name;
            let _ = write!(
                s,
                "        check!(\"{n}::{rn}\", || {{ let _ = a.{rn}(); }});\n"
            );
        }
        s.push_str("    });\n");
        statements.push(s);
    }

    // Splitting the statements keeps every generated function small enough that
    // rustc's per-function analysis stays linear.  16 statements is roughly 50
    // checks and measured fastest: below that the extra functions stop paying
    // for themselves, above it the super-linear cost creeps back in.
    const STATEMENTS_PER_SECTION: usize = 16;

    let mut out = String::new();
    let _ = writeln!(out, "// Generated by vtk-wrap-gen. Do not edit.");
    let _ = writeln!(
        out,
        "// Crash-safety checks for the `{module}` VTK module.\n"
    );
    let _ = writeln!(out, "use vtk_wrap::*;\n");

    let sections = statements.len().div_ceil(STATEMENTS_PER_SECTION);
    for (index, chunk) in statements.chunks(STATEMENTS_PER_SECTION).enumerate() {
        let _ = write!(out, "#[inline(never)]\nfn section_{index}() {{\n");
        for statement in chunk {
            out.push_str(statement);
        }
        out.push_str("}\n\n");
    }

    out.push_str("/// Run every check for this module.\npub fn run() {\n");
    for index in 0..sections {
        let _ = writeln!(out, "    section_{index}();");
    }
    out.push_str("}\n");
    out
}

/// The smoke test's crate root: the counters, the `check!` macro and a `mod`
/// per module, each `#[cfg]`-gated so a disabled module is not even parsed.
pub fn emit_smoke_index(modules: &[String]) -> String {
    let mut out = String::new();
    out.push_str(
        r##"// Generated by vtk-wrap-gen. Do not edit.
//!
//! Crash-safety smoke test over the whole wrapped API.
//!
//! Constructs every constructable class and checks the wrapper invariants
//! (instance-of relationship, `vtkObjectBase` derivation, reference-count round
//! trip), then checks the static type information of every wrapped class.
//!
//! The last line printed before a crash names the offending class.
//!
//! Each VTK module gets its own `mod`, `include!`d only when its Cargo feature
//! is on, and its checks live in `section_*` functions rather than one giant
//! `main`: rustc's type checking is super-linear in the size of a single
//! function, and a `#[cfg]` on every check is super-linear in their number.
//! `VTK_WRAP_SKIP`, the crash-hunt hook, works unchanged.

// The per-module `mod m_vtkFoo` names mirror the VTK modules.
#![allow(non_snake_case)]

use std::process::ExitCode;
use std::sync::atomic::{AtomicUsize, Ordering};

static CHECKS: AtomicUsize = AtomicUsize::new(0);
static FAILURES: AtomicUsize = AtomicUsize::new(0);

macro_rules! check {
    ($name:expr, $body:expr) => {{
        let __name: &str = $name;
        // Crash hunting: set VTK_WRAP_SKIP to a comma separated list of check
        // names to skip, so a driver script can walk past a segfault one
        // check at a time and collect the whole list of crashing calls.
        let __skip = std::env::var("VTK_WRAP_SKIP")
            .map(|s| s.split(',').any(|p| p.trim() == __name))
            .unwrap_or(false);
        if !__skip {
            print!("  {}\n", __name);
            use std::io::Write;
            let _ = std::io::stdout().flush();
            $crate::CHECKS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            match std::panic::catch_unwind(std::panic::AssertUnwindSafe($body)) {
                Ok(()) => {}
                Err(_) => {
                    $crate::FAILURES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    println!("    ^^^ FAILED");
                }
            }
        }
    }};
}

"##,
    );

    for module in modules {
        let _ = writeln!(out, "#[cfg(feature = \"{module}\")]");
        let _ = writeln!(
            out,
            "mod m_{module} {{\n    include!(\"smoke/{module}.rs\");\n}}\n"
        );
    }

    out.push_str("fn main() -> ExitCode {\n");
    for module in modules {
        let _ = writeln!(out, "    #[cfg(feature = \"{module}\")]");
        let _ = writeln!(out, "    m_{module}::run();");
    }
    out.push_str(
        // Kept in rustfmt's own shape so `cargo fmt` is a no-op on the
        // generated index (the `include!`d module files are not visited).
        "    let checks = CHECKS.load(Ordering::Relaxed);\n    let failures = FAILURES.load(Ordering::Relaxed);\n    println!(\"\\n{} checks, {} failures\", checks, failures);\n    if failures == 0 {\n        ExitCode::SUCCESS\n    } else {\n        ExitCode::FAILURE\n    }\n}\n",
    );
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
        "modules: {}\nclasses: {}\nconstructable: {}\nmethods: {}\nskipped methods (unsupported types): {}\nskipped classes (deprecated in VTK): {}\nskipped classes (wrapper-only VTK shims): {}",
        api.modules.len(),
        api.classes.len(),
        constructable,
        methods,
        api.skipped_methods,
        api.skipped_deprecated_classes,
        api.skipped_wrapper_shims
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn method(rust_name: &str, params: usize) -> Method {
        Method {
            cxx_name: rust_name.to_owned(),
            c_name: format!("vtk_wrap_{rust_name}"),
            rust_name: rust_name.to_owned(),
            is_static: false,
            params: (0..params)
                .map(|i| Param {
                    name: format!("p{i}"),
                    ty: Ty::I32,
                    default: None,
                })
                .collect(),
            ret: None,
            doc: None,
        }
    }

    fn class(name: &str) -> Class {
        Class {
            name: name.to_owned(),
            module: "vtkTest".to_owned(),
            header: format!("{name}.h"),
            is_abstract: false,
            base: None,
            deref_target: None,
            construct_expr: Some(format!("{name}::New()")),
            methods: vec![
                method("is_type_of", 1),
                method("is_a", 1),
                method("get_value", 0),
            ],
            doc: None,
        }
    }

    fn api(classes: usize) -> Api {
        let mut map = BTreeMap::new();
        for i in 0..classes {
            let name = format!("vtkTest{i:03}");
            map.insert(name.clone(), class(&name));
        }
        Api {
            classes: map,
            ..Api::default()
        }
    }

    /// Generated code is gated per *file*, not per item: a `#[cfg]` on every
    /// one of the ~35k declarations costs minutes of compile time.
    #[test]
    fn generated_files_carry_no_per_item_cfg() {
        let api = api(40);
        let modules = vec!["vtkTest".to_owned()];
        for (label, text) in [
            ("ffi", emit_ffi_module(&api, "vtkTest")),
            ("wrappers", emit_wrappers_module(&api, "vtkTest")),
            ("smoke", emit_smoke_module(&api, "vtkTest")),
        ] {
            assert!(
                !text.contains("#[cfg(feature"),
                "{label} module file must not contain per-item cfg"
            );
        }
        // The index files carry exactly one `#[cfg]` per module.
        assert_eq!(
            emit_wrappers_index(&modules)
                .matches("#[cfg(feature")
                .count(),
            1
        );
        assert_eq!(emit_ffi_index(&modules).matches("#[cfg(feature").count(), 1);
        assert_eq!(
            emit_smoke_index(&modules).matches("#[cfg(feature").count(),
            2, // the `mod` and its call in `main`
        );
    }

    /// Every module file is `include!`d, and every module's `run` is called.
    #[test]
    fn indexes_include_and_call_every_module() {
        let modules = vec!["vtkCommonCore".to_owned(), "vtkFiltersCore".to_owned()];
        let wrappers = emit_wrappers_index(&modules);
        let ffi = emit_ffi_index(&modules);
        let smoke = emit_smoke_index(&modules);
        for module in &modules {
            assert!(
                wrappers.contains(&format!("include!(\"generated/{module}.rs\")")),
                "{wrappers}"
            );
            assert!(
                ffi.contains(&format!("include!(\"generated/{module}.rs\")")),
                "{ffi}"
            );
            assert!(
                smoke.contains(&format!("include!(\"smoke/{module}.rs\")")),
                "{smoke}"
            );
            assert!(smoke.contains(&format!("m_{module}::run();")), "{smoke}");
        }
    }

    /// The smoke test must be split into `section_*` functions (a single giant
    /// function makes rustc's type checking super-linear), all called in order
    /// from `run`.
    #[test]
    fn smoke_test_is_split_into_called_sections() {
        let api = api(3);
        let out = emit_smoke_module(&api, "vtkTest");

        // 3 classes * (is_type_of + new + accessor sweep) = 9 statements, which
        // fits in one section.
        assert!(out.contains("fn section_0() {"), "{out}");
        assert!(!out.contains("fn section_1()"), "{out}");
        assert!(out.contains("pub fn run() {"), "{out}");
        assert!(out.contains("    section_0();\n"), "{out}");

        // The checks themselves are unchanged.
        assert!(out.matches("check!(").count() >= 3 * 3, "{out}");
        assert!(out.contains("vtkTest000::is_type_of"), "{out}");
        assert!(out.contains("vtkTest000::new"), "{out}");
        assert!(out.contains("vtkTest000::get_value"), "{out}");
    }

    #[test]
    fn smoke_test_splits_once_a_section_is_full() {
        // 40 classes * 3 = 120 statements / 16 per section = 8 sections.
        let out = emit_smoke_module(&api(40), "vtkTest");
        let sections = out.matches("fn section_").count();
        assert_eq!(sections, 8, "unexpected section count");
        // `run` calls every section exactly once.
        for i in 0..sections {
            assert_eq!(
                out.matches(&format!("    section_{i}();")).count(),
                1,
                "section_{i} not called exactly once"
            );
        }
    }
}
