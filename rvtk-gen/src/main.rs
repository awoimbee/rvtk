//! `rvtk-gen` turns the XML API description produced by
//! [WrapVTK](https://github.com/dgobbi/WrapVTK)'s `vtkWrapXML` tool into a
//! C++ shim, raw Rust FFI declarations and safe Rust wrappers.
//!
//! Run it against a directory of generated XML, for example:
//!
//! ```text
//! rvtk-gen \
//!   --xml-dir /tmp/WrapVTK/build/xml \
//!   --repo . \
//!   --modules vtkCommonCore,vtkCommonDataModel,vtkFiltersSources
//! ```

#![allow(dead_code)]

mod gen_cpp;
mod gen_rust;
mod model;
mod parse;

use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::Parser;

#[derive(Parser, Debug)]
#[command(
    name = "rvtk-gen",
    about = "Generate Rust bindings from VTK WrapVTK XML"
)]
struct Args {
    /// Directory containing one sub-directory of `*.xml` files per VTK module.
    #[arg(long)]
    xml_dir: PathBuf,

    /// Root of the repository; output is written below it.
    #[arg(long, default_value = ".")]
    repo: PathBuf,

    /// Comma separated list of VTK modules to wrap (default: all found).
    #[arg(long)]
    modules: Option<String>,

    /// VTK include directory (e.g. `$VTK/include/vtk-9.7`).
    ///
    /// Used to recognise VTK's "fake superclass" array shims, which declare an
    /// interface that only exists when `__VTK_WRAP__` is defined and which
    /// segfault inside VTK when called from ordinary C++.
    #[arg(long)]
    vtk_include: Option<PathBuf>,
}

fn main() -> Result<()> {
    let args = Args::parse();

    let modules: Vec<String> = args
        .modules
        .as_deref()
        .map(|s| {
            s.split(',')
                .map(|m| m.trim().to_string())
                .filter(|m| !m.is_empty())
                .collect()
        })
        .unwrap_or_default();

    let api = parse::build_api(
        &args.xml_dir,
        &parse::BuildOptions {
            modules,
            vtk_include: args.vtk_include.clone(),
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

    let ffi = gen_rust::emit_ffi(&api);
    std::fs::write(args.repo.join("rvtk-sys/src/generated.rs"), ffi)?;

    let wrappers = gen_rust::emit_wrappers(&api);
    std::fs::write(args.repo.join("rvtk/src/generated.rs"), wrappers)?;

    let smoke = gen_rust::emit_smoke_test(&api);
    std::fs::write(args.repo.join("rvtk/tests/smoke.rs"), smoke)?;

    println!("{}", gen_rust::summary(&api));
    Ok(())
}
