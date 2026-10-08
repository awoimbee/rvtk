//! `rvtk-gen` turns the XML API description produced by
//! [WrapVTK](https://github.com/dgobbi/WrapVTK)'s `vtkWrapXML` tool into a
//! C++ shim, raw Rust FFI declarations and safe Rust wrappers.
//!
//! It can produce that XML itself: point it at VTK and it clones (or reuses)
//! WrapVTK, builds `vtkWrapXML`, runs it, and then generates the bindings.  A
//! directory of pre-generated XML can be supplied instead with `--xml-dir`.
//!
//! ```text
//! # End to end, from an installed VTK:
//! rvtk-gen --repo . --vtk-dir /usr/lib/cmake/vtk-9.7
//!
//! # From XML that already exists:
//! rvtk-gen --repo . --xml-dir /tmp/WrapVTK/build/xml \
//!   --modules vtkCommonCore,vtkCommonDataModel,vtkFiltersSources
//! ```

#![allow(dead_code)]

mod gen_cpp;
mod gen_rust;
mod model;
mod parse;
mod wrapvtk;

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use clap::Parser;

#[derive(Parser, Debug)]
#[command(
    name = "rvtk-gen",
    about = "Generate Rust bindings from VTK's wrapping metadata"
)]
struct Args {
    /// Directory containing one sub-directory of `*.xml` files per VTK module.
    ///
    /// When omitted, the XML is produced first: WrapVTK is cloned and built
    /// against `--vtk-dir`, and `vtkWrapXML` is run over `--modules`.
    #[arg(long)]
    xml_dir: Option<PathBuf>,

    /// Root of the repository; output is written below it.
    #[arg(long, default_value = ".")]
    repo: PathBuf,

    /// Comma separated list of VTK modules to wrap (default: all found).
    ///
    /// Names use VTK's spelling (`vtkCommonCore`); the `vtk` prefix is optional
    /// and is stripped when talking to WrapVTK.
    #[arg(long)]
    modules: Option<String>,

    /// VTK CMake package directory (the one holding `vtk-config.cmake`), or an
    /// install prefix.  Needed to build `vtkWrapXML` unless `--xml-dir` is
    /// given; `VTK_DIR` and a Homebrew install are used as fallbacks.
    #[arg(long)]
    vtk_dir: Option<PathBuf>,

    /// VTK include directory (e.g. `$VTK/include/vtk-9.7`).
    ///
    /// Used to recognise VTK's "fake superclass" array shims, which declare an
    /// interface that only exists when `__VTK_WRAP__` is defined and which
    /// segfault inside VTK when called from ordinary C++.  Derived from
    /// `--vtk-dir` when possible.
    #[arg(long)]
    vtk_include: Option<PathBuf>,

    /// WrapVTK checkout to use or create (default: `<repo>/target/wrapvtk`).
    #[arg(long)]
    wrapvtk_dir: Option<PathBuf>,

    /// Git URL to clone WrapVTK from.
    #[arg(long)]
    wrapvtk_url: Option<String>,

    /// Passed to `cmake --build --parallel` when building WrapVTK.
    #[arg(long)]
    jobs: Option<String>,
}

fn main() -> Result<()> {
    let args = Args::parse();

    let modules = split_modules(args.modules.as_deref());
    // Without an explicit list, wrap exactly the modules the crate exposes as
    // Cargo features (see `rvtk-sys/Cargo.toml`), so the generated XML, the
    // bindings and the features stay in step.
    let modules = if modules.is_empty() {
        wrapvtk::modules_from_features(&args.repo)
    } else {
        modules
    };

    // Either use the XML we were handed, or produce it with WrapVTK.
    let (xml_dir, vtk_dir) = match &args.xml_dir {
        Some(dir) => (dir.clone(), args.vtk_dir.clone()),
        None => {
            let request = wrapvtk::Request {
                repo: args.repo.clone(),
                vtk_dir: args.vtk_dir.clone(),
                wrapvtk_dir: args.wrapvtk_dir.clone(),
                wrapvtk_url: args
                    .wrapvtk_url
                    .clone()
                    .unwrap_or_else(|| wrapvtk::DEFAULT_WRAPVTK_URL.to_owned()),
                modules: modules.clone(),
                jobs: args.jobs.clone(),
            };
            let xml = wrapvtk::ensure_xml(&request)?;
            (xml, request.vtk_dir)
        }
    };

    // `vtkWrapXML` marks certain array shims based on the headers, so try to
    // find the include directory even when it was not passed explicitly.  When
    // VTK is being wrapped anyway its location is already known; otherwise it
    // is worth a best-effort look, because missing it silently adds ~70
    // wrapper-only classes to the generated bindings.
    let vtk_include = args
        .vtk_include
        .clone()
        .or_else(|| std::env::var_os("VTK_INCLUDE").map(PathBuf::from))
        .or_else(|| vtk_dir.as_deref().and_then(wrapvtk::vtk_include_dir))
        .or_else(|| {
            let detected = wrapvtk::resolve_vtk_dir(None, &args.repo).ok()?;
            wrapvtk::vtk_include_dir(&detected)
        });

    let api = parse::build_api(
        &xml_dir,
        &parse::BuildOptions {
            modules,
            vtk_include,
        },
    )?;

    let shim = args.repo.join("rvtk-sys/shim");
    let shim_src = shim.join("src");
    std::fs::create_dir_all(&shim_src)
        .with_context(|| format!("creating {}", shim_src.display()))?;
    std::fs::create_dir_all(shim.join("support"))?;

    // Remove previously generated translation units.
    for entry in std::fs::read_dir(&shim_src)?.filter_map(|e| e.ok()) {
        let path = entry.path();
        if path.extension().map(|e| e == "cpp").unwrap_or(false) {
            let _ = std::fs::remove_file(path);
        }
    }

    let mut generated = Vec::new();
    for class in api.classes.values() {
        let cpp = gen_cpp::emit_class_cpp(class, &api.headers);
        let file = shim_src.join(format!("{}.cpp", class.name));
        std::fs::write(&file, cpp).with_context(|| format!("writing {}", file.display()))?;
        generated.push((class.name.clone(), Vec::<String>::new()));
    }

    let cmake = gen_cpp::emit_cmake(&api, &generated);
    std::fs::write(shim.join("CMakeLists.txt"), cmake)?;

    // One file per VTK module, plus an index that includes them behind their
    // Cargo features.  Splitting the files, rather than gating every item with
    // `#[cfg]`, is what keeps the generated code cheap to compile: rustc's cost
    // is super-linear in the number of gated items.
    let modules = &api.modules;

    write_modules(&args.repo.join("rvtk-sys/src/generated"), modules, |module| {
        gen_rust::emit_ffi_module(&api, module)
    })?;
    std::fs::write(
        args.repo.join("rvtk-sys/src/generated.rs"),
        gen_rust::emit_ffi_index(modules),
    )?;

    write_modules(&args.repo.join("rvtk/src/generated"), modules, |module| {
        gen_rust::emit_wrappers_module(&api, module)
    })?;
    std::fs::write(
        args.repo.join("rvtk/src/generated.rs"),
        gen_rust::emit_wrappers_index(modules),
    )?;

    write_modules(&args.repo.join("rvtk/tests/smoke"), modules, |module| {
        gen_rust::emit_smoke_module(&api, module)
    })?;
    std::fs::write(
        args.repo.join("rvtk/tests/smoke.rs"),
        gen_rust::emit_smoke_index(modules),
    )?;

    println!("{}", gen_rust::summary(&api));
    Ok(())
}

/// Write one generated `.rs` file per module into `dir`, removing stale ones
/// first so a module that is no longer wrapped does not linger.
fn write_modules<F>(dir: &Path, modules: &[String], mut emit: F) -> Result<()>
where
    F: FnMut(&str) -> String,
{
    std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    for entry in std::fs::read_dir(dir)?.filter_map(|e| e.ok()) {
        let path = entry.path();
        if path.extension().map(|e| e == "rs").unwrap_or(false) {
            let _ = std::fs::remove_file(path);
        }
    }
    for module in modules {
        let path = dir.join(format!("{module}.rs"));
        std::fs::write(&path, emit(module))
            .with_context(|| format!("writing {}", path.display()))?;
    }
    Ok(())
}

/// Split a comma separated module list, dropping blanks and the `vtk` prefix is
/// left intact (VTK's spelling is what the XML directories and features use).
fn split_modules(modules: Option<&str>) -> Vec<String> {
    modules
        .map(|list| {
            list.split(',')
                .map(|module| module.trim().to_string())
                .filter(|module| !module.is_empty())
                .collect()
        })
        .unwrap_or_default()
}
