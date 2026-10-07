//! Parser for the XML produced by WrapVTK's `vtkWrapXML` tool.
//!
//! The XML format is documented in WrapVTK's `Documentation/WrapVTK_XML.md`.
//! We parse the (large) collection of per-class files into a small intermediate
//! representation, dropping anything that cannot be represented across a C ABI.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::Path;

use anyhow::{Context, Result};

use crate::model::{Api, Class, Method, Param, Ty};

#[derive(Debug, Default, Clone)]
struct RawParam {
    name: Option<String>,
    type_: Option<String>,
    pointer: Option<String>,
    reference: bool,
    size: Option<String>,
    default: Option<String>,
    rvalue: bool,
    pack: bool,
    /// true when the param has a nested <function>/<method> child (function ptr)
    is_function: bool,
}

#[derive(Debug, Default, Clone)]
struct RawMethod {
    name: String,
    access: String,
    context: Option<String>,
    is_static: bool,
    is_template: bool,
    deprecated: bool,
    wrapexclude: bool,
    params: Vec<RawParam>,
    ret: RawParam,
    doc: Option<String>,
}

#[derive(Debug, Clone)]
struct RawClass {
    name: String,
    module: String,
    header: String,
    is_abstract: bool,
    is_template: bool,
    bases: Vec<String>,
    inheritance: Vec<String>,
    has_public_ctor: bool,
    ctor_all_defaulted: bool,
    doc: Option<String>,
    property_size: BTreeMap<String, usize>,
    methods: Vec<RawMethod>,
}

fn attr<'a>(node: &'a roxmltree::Node, name: &str) -> Option<&'a str> {
    node.attribute(name)
}

fn bool_attr(node: &roxmltree::Node, name: &str) -> bool {
    attr(node, name) == Some("1")
}

fn child_text(node: &roxmltree::Node, tag: &str) -> Option<String> {
    let text = node
        .children()
        .find(|c| c.has_tag_name(tag))
        .map(|c| c.text().unwrap_or("").trim().to_string())?;
    if text.is_empty() {
        None
    } else {
        Some(text)
    }
}

fn parse_param(node: &roxmltree::Node) -> RawParam {
    RawParam {
        name: attr(node, "name").map(str::to_string),
        type_: attr(node, "type").map(str::to_string),
        pointer: attr(node, "pointer").map(str::to_string),
        reference: bool_attr(node, "reference"),
        size: attr(node, "size").map(str::to_string),
        default: attr(node, "value").map(str::to_string),
        rvalue: bool_attr(node, "rvalue_reference"),
        pack: bool_attr(node, "pack"),
        is_function: node
            .children()
            .any(|c| c.has_tag_name("function") || c.has_tag_name("method")),
    }
}

fn parse_property_size(raw: &str) -> Option<usize> {
    match raw {
        ":" => None,
        s if s.starts_with('{') => {
            let inner = s.trim_start_matches('{').trim_end_matches('}');
            inner.split(',').try_fold(1usize, |acc, p| {
                p.trim().parse::<usize>().ok().map(|v| acc * v)
            })
        }
        s => s.parse::<usize>().ok(),
    }
}

fn parse_class(node: &roxmltree::Node, module: &str, header: &str) -> RawClass {
    let name = attr(node, "name").unwrap_or_default().to_string();
    let mut bases = Vec::new();
    let mut inheritance = Vec::new();
    let mut property_size = BTreeMap::new();
    let mut methods = Vec::new();
    let mut has_public_ctor = false;
    let mut ctor_all_defaulted = true;

    for child in node.children().filter(|c| c.is_element()) {
        match child.tag_name().name() {
            "base" => {
                if let Some(b) = attr(&child, "name") {
                    bases.push(b.to_string());
                }
            }
            "inheritance" => {
                for ctx in child.children().filter(|c| c.has_tag_name("context")) {
                    if let Some(n) = attr(&ctx, "name") {
                        inheritance.push(n.to_string());
                    }
                }
            }
            "property" => {
                if let (Some(pname), Some(size)) = (attr(&child, "name"), attr(&child, "size")) {
                    if let Some(n) = parse_property_size(size) {
                        property_size.insert(pname.to_string(), n);
                    }
                }
            }
            "constructor" => {
                let access = attr(&child, "access").unwrap_or("public");
                if access == "public" {
                    has_public_ctor = true;
                    if !child
                        .children()
                        .filter(|c| c.has_tag_name("param"))
                        .all(|c| attr(&c, "value").is_some())
                    {
                        ctor_all_defaulted = false;
                    }
                }
            }
            "method" => {
                let ret = child
                    .children()
                    .find(|c| c.has_tag_name("return"))
                    .as_ref()
                    .map(parse_param)
                    .unwrap_or_default();
                let params = child
                    .children()
                    .filter(|c| c.has_tag_name("param"))
                    .map(|p| parse_param(&p))
                    .collect();
                methods.push(RawMethod {
                    name: attr(&child, "name").unwrap_or_default().to_string(),
                    access: attr(&child, "access").unwrap_or("public").to_string(),
                    context: attr(&child, "context").map(str::to_string),
                    is_static: bool_attr(&child, "static"),
                    is_template: bool_attr(&child, "template"),
                    deprecated: bool_attr(&child, "deprecated"),
                    wrapexclude: bool_attr(&child, "wrapexclude"),
                    params,
                    ret,
                    doc: child_text(&child, "comment"),
                });
            }
            _ => {}
        }
    }

    RawClass {
        name,
        module: module.to_string(),
        header: header.to_string(),
        is_abstract: bool_attr(node, "abstract"),
        is_template: bool_attr(node, "template"),
        bases,
        inheritance,
        has_public_ctor,
        ctor_all_defaulted,
        doc: child_text(node, "comment"),
        property_size,
        methods,
    }
}

/// Parse every `*.xml` file below `xml_dir`, keeping only the modules selected
/// by `modules` (empty means all).
fn parse_dir<P: AsRef<Path>>(xml_dir: P, modules: &[String]) -> Result<Vec<RawClass>> {
    let xml_dir = xml_dir.as_ref();
    let mut out = Vec::new();
    let mut entries: Vec<_> = std::fs::read_dir(xml_dir)
        .with_context(|| format!("reading {}", xml_dir.display()))?
        .filter_map(|e| e.ok())
        .collect();
    entries.sort_by_key(|e| e.file_name());

    for entry in entries {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let module = entry.file_name().to_string_lossy().to_string();
        if !modules.is_empty() && !modules.iter().any(|m| m == &module) {
            continue;
        }
        let mut files: Vec<_> = std::fs::read_dir(&path)?
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.extension().map(|e| e == "xml").unwrap_or(false))
            .collect();
        files.sort();
        for file in files {
            let text = std::fs::read_to_string(&file)
                .with_context(|| format!("reading {}", file.display()))?;
            let doc = roxmltree::Document::parse(&text)
                .with_context(|| format!("parsing {}", file.display()))?;
            let root = doc.root_element();
            let header = attr(&root, "name").unwrap_or("").to_string();
            for class in root.children().filter(|c| c.has_tag_name("class")) {
                if class.attribute("access").is_some() {
                    continue; // nested class
                }
                let raw = parse_class(&class, &module, &header);
                if !raw.name.is_empty() {
                    out.push(raw);
                }
            }
        }
    }
    Ok(out)
}

/// The set of class names that derive from `vtkObjectBase` (directly or not).
fn refcounted_set(raws: &[RawClass]) -> BTreeSet<String> {
    let mut inherits: HashMap<&str, bool> = HashMap::new();
    for c in raws {
        let direct = c.inheritance.iter().any(|n| n == "vtkObjectBase")
            || c.bases.iter().any(|n| n == "vtkObjectBase")
            || c.name == "vtkObjectBase";
        inherits.insert(&c.name, direct);
    }
    loop {
        let mut changed = false;
        for c in raws {
            if *inherits.get(c.name.as_str()).unwrap_or(&false) {
                continue;
            }
            if c.bases
                .iter()
                .any(|b| *inherits.get(b.as_str()).unwrap_or(&false))
            {
                inherits.insert(&c.name, true);
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    inherits
        .into_iter()
        .filter(|(_, v)| *v)
        .map(|(k, _)| k.to_string())
        .collect()
}

const SCALAR_MAP: &[(&str, Ty)] = &[
    ("bool", Ty::Bool),
    ("char", Ty::CChar),
    ("signed char", Ty::I8),
    ("unsigned char", Ty::U8),
    ("short", Ty::I16),
    ("short int", Ty::I16),
    ("unsigned short", Ty::U16),
    ("unsigned short int", Ty::U16),
    ("int", Ty::I32),
    ("unsigned int", Ty::U32),
    ("unsigned", Ty::U32),
    ("long", Ty::CLong),
    ("long int", Ty::CLong),
    ("unsigned long", Ty::CULong),
    ("unsigned long int", Ty::CULong),
    ("long long", Ty::I64),
    ("long long int", Ty::I64),
    ("unsigned long long", Ty::U64),
    ("unsigned long long int", Ty::U64),
    ("float", Ty::F32),
    ("double", Ty::F64),
    ("size_t", Ty::Usize),
    ("ssize_t", Ty::Isize),
    ("ptrdiff_t", Ty::Isize),
    ("vtkIdType", Ty::I64),
    ("vtkMTimeType", Ty::I64),
    ("vtkTypeInt64", Ty::I64),
    ("vtkTypeUInt64", Ty::U64),
    ("vtkTypeBool", Ty::I32),
    ("vtkTypeInt32", Ty::I32),
    ("vtkTypeUInt32", Ty::U32),
    ("vtkTypeInt8", Ty::I8),
    ("vtkTypeUInt8", Ty::U8),
    ("vtkTypeInt16", Ty::I16),
    ("vtkTypeUInt16", Ty::U16),
    ("vtkTypeFloat32", Ty::F32),
    ("vtkTypeFloat64", Ty::F64),
];

struct TypeCtx<'a> {
    known_classes: &'a BTreeSet<String>,
}

/// Resolve a parameter/return type.  `type_ == None` or `void` without a pointer
/// means "no value".  Returns `Err(())` when the type is unsupported.
fn resolve_ty(
    type_: Option<&str>,
    pointer: Option<&str>,
    size: Option<&str>,
    ctx: &TypeCtx,
    is_return: bool,
) -> std::result::Result<Option<Ty>, ()> {
    let Some(raw) = type_ else {
        return Ok(None);
    };
    let mut base = raw.trim();
    let mut is_const = false;
    while let Some(rest) = base.strip_prefix("const ") {
        is_const = true;
        base = rest.trim();
    }
    let has_ptr = pointer.map(|p| p.contains('*')).unwrap_or(false);
    let ptr_depth = pointer.map(|p| p.matches('*').count()).unwrap_or(0);

    if base == "void" {
        if ptr_depth == 1 {
            return Ok(Some(Ty::RawPtr));
        } else if ptr_depth > 1 {
            return Err(());
        }
        return Ok(None);
    }

    if base == "char" && has_ptr {
        // `const char *x[]` (depth 2) is an array of strings, `char *` is an
        // output buffer of unknown length; only a plain `const char *` works.
        if !is_const || ptr_depth != 1 || size.is_some() {
            return Err(());
        }
        return Ok(Some(Ty::CStr));
    }

    if matches!(base, "std::string" | "vtkStdString" | "string") {
        // A `std::string` behind a pointer/reference cannot be represented.
        if has_ptr {
            return Err(());
        }
        return Ok(Some(Ty::StdString));
    }

    for (name, ty) in SCALAR_MAP {
        if *name == base {
            if let Some(size_str) = size {
                if size_str == ":" {
                    if is_return {
                        return Err(()); // variable length arrays cannot be returned
                    }
                    return Ok(Some(Ty::Slice(Box::new(ty.clone()), is_const)));
                }
                if size_str.starts_with('{') {
                    return Err(()); // multi dimensional arrays are not supported
                }
                if let Some(n) = parse_property_size(size_str) {
                    return Ok(Some(Ty::Array(Box::new(ty.clone()), n, is_const)));
                }
            }
            if has_ptr {
                return Err(()); // pointer to scalar without a known extent
            }
            return Ok(Some(ty.clone()));
        }
    }

    // `vtkSmartPointer<T>` is unwrapped to a plain object pointer.
    if let Some(rest) = base.strip_prefix("vtkSmartPointer<") {
        let inner = rest.strip_suffix('>').ok_or(())?.trim();
        if ctx.known_classes.contains(inner) {
            return Ok(Some(Ty::SmartObject(inner.to_string())));
        }
        return Err(());
    }

    // refcounted class behind a single pointer
    if pointer == Some("*") && ctx.known_classes.contains(base) {
        // `T *x[]` (an array of pointers) carries a `size` attribute and decays
        // to `T **`, which we do not support.
        if size.is_some() {
            return Err(());
        }
        return Ok(Some(Ty::Object(base.to_string())));
    }

    Err(())
}

pub struct BuildOptions {
    pub modules: Vec<String>,
}

pub fn build_api(xml_dir: &Path, opts: &BuildOptions) -> Result<Api> {
    let raws = parse_dir(xml_dir, &opts.modules)?;
    let refcounted = refcounted_set(&raws);

    // Classes we actually emit (`refcounted`, not templates).
    let wrapped: BTreeSet<String> = raws
        .iter()
        .filter(|c| refcounted.contains(&c.name) && !c.is_template)
        .map(|c| c.name.clone())
        .collect();
    let ctx = TypeCtx {
        known_classes: &wrapped,
    };

    let mut api = Api::default();
    let mut modules: BTreeSet<String> = BTreeSet::new();

    for raw in &raws {
        if !wrapped.contains(&raw.name) {
            continue;
        }
        modules.insert(raw.module.clone());

        // nearest wrapped base (walk the inheritance chain nearest-first)
        let deref_target = raw
            .inheritance
            .iter()
            .find(|n| *n != &raw.name && wrapped.contains(*n))
            .cloned()
            .or_else(|| raw.bases.iter().find(|n| wrapped.contains(*n)).cloned());

        // Construction.
        let mut construct_expr = None;
        for m in &raw.methods {
            if m.name == "New"
                && m.is_static
                && m.access == "public"
                && m.params.is_empty()
                && !m.is_template
                && m.ret.type_.as_deref() == Some(raw.name.as_str())
                && m.ret.pointer.as_deref() == Some("*")
            {
                construct_expr = Some(format!("{}::New()", raw.name));
                break;
            }
        }
        if construct_expr.is_none() && raw.has_public_ctor && raw.ctor_all_defaulted {
            construct_expr = Some(format!("new {}()", raw.name));
        }

        struct Candidate {
            method: Method,
            base_name: String,
            param_count: usize,
            order: usize,
        }
        let mut candidates: Vec<Candidate> = Vec::new();
        let mut group_counter: BTreeMap<String, usize> = BTreeMap::new();

        for rm in &raw.methods {
            if rm
                .context
                .as_deref()
                .map(|c| c != raw.name)
                .unwrap_or(false)
            {
                continue;
            }
            if rm.access != "public" || rm.is_template || rm.wrapexclude || rm.deprecated {
                continue;
            }
            if matches!(
                rm.name.as_str(),
                "New" | "NewInstance" | "PrintSelf" | "GetClassNameInternal"
            ) {
                continue;
            }
            if rm
                .params
                .iter()
                .any(|p| p.rvalue || p.pack || p.is_function)
                || rm.ret.is_function
            {
                continue;
            }
            if rm
                .params
                .iter()
                .any(|p| p.reference && p.type_.as_deref() != Some("const std::string"))
            {
                continue;
            }

            let ret = match resolve_ty(
                rm.ret.type_.as_deref(),
                rm.ret.pointer.as_deref(),
                rm.ret.size.as_deref(),
                &ctx,
                true,
            ) {
                Ok(t) => t,
                Err(()) => {
                    api.skipped_methods += 1;
                    continue;
                }
            };

            let mut params = Vec::new();
            let mut ok = true;
            for (i, p) in rm.params.iter().enumerate() {
                match resolve_ty(
                    p.type_.as_deref(),
                    p.pointer.as_deref(),
                    p.size.as_deref(),
                    &ctx,
                    false,
                ) {
                    Ok(Some(t)) => {
                        let name = p
                            .name
                            .clone()
                            .filter(|n| !n.is_empty())
                            .unwrap_or_else(|| format!("arg{i}"));
                        params.push(Param {
                            name: sanitize_ident(&name),
                            ty: t,
                            default: p.default.clone(),
                        });
                    }
                    _ => {
                        ok = false;
                        break;
                    }
                }
            }
            if !ok {
                api.skipped_methods += 1;
                continue;
            }

            let base_name = to_rust_name(&rm.name);
            let occ = {
                let e = group_counter.entry(rm.name.clone()).or_insert(0);
                let v = *e;
                *e += 1;
                v
            };
            let param_count = params.len();
            candidates.push(Candidate {
                base_name,
                param_count,
                order: candidates.len(),
                method: Method {
                    cxx_name: rm.name.clone(),
                    c_name: format!("{}__{}__{}", raw.name, rm.name, occ),
                    rust_name: String::new(),
                    is_static: rm.is_static,
                    params,
                    ret,
                    doc: rm.doc.clone(),
                },
            });
        }

        // Disambiguate overloads.  The overload with the fewest parameters
        // keeps the plain name (so `update()` beats `update(port)`), the others
        // get a `_vN` suffix.
        let mut groups: BTreeMap<String, Vec<usize>> = BTreeMap::new();
        for (i, c) in candidates.iter().enumerate() {
            groups.entry(c.base_name.clone()).or_default().push(i);
        }
        let mut used_rust_names: BTreeSet<String> = BTreeSet::new();
        for (base, mut idxs) in groups {
            idxs.sort_by_key(|&i| {
                (
                    candidates[i].param_count,
                    overload_penalty(&candidates[i].method.params),
                    candidates[i].order,
                )
            });
            for (rank, &i) in idxs.iter().enumerate() {
                let mut name = if rank == 0 {
                    base.clone()
                } else {
                    format!("{base}_v{}", rank + 1)
                };
                while used_rust_names.contains(&name) {
                    name.push('_');
                }
                used_rust_names.insert(name.clone());
                candidates[i].method.rust_name = name;
            }
        }
        let methods: Vec<Method> = candidates.into_iter().map(|c| c.method).collect();

        let _ = raw.property_size; // reserved for future array-return handling
        api.classes.insert(
            raw.name.clone(),
            Class {
                name: raw.name.clone(),
                module: raw.module.clone(),
                header: raw.header.clone(),
                is_abstract: raw.is_abstract,
                base: deref_target.clone(),
                deref_target,
                construct_expr,
                methods,
                doc: raw.doc.clone(),
            },
        );
    }

    api.modules = modules.into_iter().collect();
    api.headers = raws
        .iter()
        .filter(|c| wrapped.contains(&c.name))
        .map(|c| (c.name.clone(), c.header.clone()))
        .collect();
    Ok(api)
}

/// A tie-breaker used when several overloads have the same arity.  Lower is
/// preferred.  We favour VTK's canonical scalar types (`double`, `int`) and
/// plain scalars over fixed size arrays, so that e.g. `vtkPoints`'s
/// `InsertNextPoint(const double[3])` beats the `const float[3]` variant.
fn overload_penalty(params: &[Param]) -> i32 {
    fn score(ty: &Ty) -> i32 {
        match ty {
            Ty::F32 | Ty::I16 | Ty::U16 | Ty::I8 | Ty::U8 => 2,
            Ty::Array(inner, ..) | Ty::Slice(inner, _) => 1 + score(inner),
            _ => 0,
        }
    }
    params.iter().map(|p| score(&p.ty)).sum()
}

pub fn to_rust_name(name: &str) -> String {
    use convert_case::{Case, Casing};
    let mut s = name.to_case(Case::Snake);
    if RUST_KEYWORDS.contains(&s.as_str()) {
        s.push('_');
    }
    s
}

fn sanitize_ident(name: &str) -> String {
    let mut s: String = name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if s.is_empty() {
        s.push_str("arg");
    }
    if s.chars().next().map(|c| c.is_numeric()).unwrap_or(false) {
        s.insert(0, '_');
    }
    if RUST_KEYWORDS.contains(&s.as_str()) {
        s.push('_');
    }
    s
}

const RUST_KEYWORDS: &[&str] = &[
    "as", "break", "const", "continue", "crate", "else", "enum", "extern", "false", "fn", "for",
    "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub", "ref", "return",
    "self", "Self", "static", "struct", "super", "trait", "true", "type", "unsafe", "use", "where",
    "while", "async", "await", "dyn", "abstract", "become", "box", "do", "final", "macro",
    "override", "priv", "typeof", "unsized", "virtual", "yield", "try",
];
