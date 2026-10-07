//! Build script for `vtk-sys`.
//!
//! It locates an installed VTK, configures the generated C++ shim with CMake and
//! links the resulting shared library.

use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=shim");
    println!("cargo:rerun-if-env-changed=VTK_DIR");
    println!("cargo:rerun-if-env-changed=VTK_VERSION");

    // docs.rs has no VTK installed; skip the native build there.
    if std::env::var("DOCS_RS").is_ok() {
        return;
    }

    let Some((vtk_dir, cmake_dir)) = find_vtk() else {
        panic!(
            "could not find an installed VTK.\n\
             Install it with `brew install vtk` (macOS), your package manager, or\n\
             point `VTK_DIR` at the directory containing `VTKConfig.cmake`."
        );
    };
    println!("cargo:warning=using VTK at {}", vtk_dir.display());

    let mut cfg = cmake::Config::new("shim");
    cfg.define("CMAKE_BUILD_TYPE", "Release");
    cfg.define("VTK_DIR", &cmake_dir);
    cfg.define("CMAKE_PREFIX_PATH", &vtk_dir);
    if let Ok(version) = std::env::var("VTK_VERSION") {
        cfg.define("RVTK_VTK_VERSION", version);
    }

    let dst = cfg.build();
    let libdir = dst.join("lib");

    println!("cargo:rustc-link-search=native={}", libdir.display());
    println!("cargo:rustc-link-lib=dylib=rvtk_shim");
    // Make sure the freshly built shim is found at run time.
    println!("cargo:rustc-link-arg=-Wl,-rpath,{}", libdir.display());
}

/// Search for VTK.  Returns `(prefix, cmake_dir)`.
fn find_vtk() -> Option<(PathBuf, PathBuf)> {
    // 1. Explicit override.
    if let Ok(dir) = std::env::var("VTK_DIR") {
        let dir = PathBuf::from(dir);
        if let Some(prefix) = infer_prefix(&dir) {
            return Some((prefix, dir));
        }
    }

    // 2. Homebrew.
    for prefix in ["/opt/homebrew", "/usr/local"] {
        let vtk_prefix = PathBuf::from(prefix).join("opt/vtk");
        if let Some(cmake_dir) = find_cmake_dir(&vtk_prefix) {
            return Some((vtk_prefix, cmake_dir));
        }
    }

    // 3. `brew --prefix vtk`.
    if let Ok(out) = Command::new("brew").args(["prefix", "vtk"]).output() {
        if out.status.success() {
            let prefix = PathBuf::from(String::from_utf8_lossy(&out.stdout).trim());
            if let Some(cmake_dir) = find_cmake_dir(&prefix) {
                return Some((prefix, cmake_dir));
            }
        }
    }

    // 4. Common system locations.
    for prefix in ["/usr", "/usr/local", "/opt/local"] {
        let p = PathBuf::from(prefix);
        if let Some(cmake_dir) = find_cmake_dir(&p) {
            return Some((p, cmake_dir));
        }
    }

    None
}

fn infer_prefix(vtk_dir: &Path) -> Option<PathBuf> {
    // VTK_DIR points at .../lib/cmake/vtk-X.Y; the prefix is three levels up.
    let prefix = vtk_dir.parent()?.parent()?.parent()?;
    Some(prefix.to_path_buf())
}

fn find_cmake_dir(prefix: &Path) -> Option<PathBuf> {
    let base = prefix.join("lib/cmake");
    let entries = std::fs::read_dir(&base).ok()?;
    let mut candidates: Vec<PathBuf> = entries
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .map(|n| n.starts_with("vtk-"))
                .unwrap_or(false)
        })
        .collect();
    candidates.sort();
    candidates
        .into_iter()
        .rev()
        .find(|c| c.join("VTKConfig.cmake").exists() || c.join("vtk-config.cmake").exists())
}
