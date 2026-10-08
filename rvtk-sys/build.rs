//! Build script for `rvtk-sys`.
//!
//! VTK is built from source, statically, and the generated C++ shim is built and
//! linked the same way, so building this crate never depends on an installed
//! (system) VTK.
//!
//! The script:
//!   1. fetches and unpacks the pinned VTK release,
//!   2. configures, builds and installs VTK as static libraries,
//!   3. builds the C++ shim against that VTK,
//!   4. tells rustc which libraries and frameworks to link.
//!
//! Steps 1 and 2 are cached under `target/`, so they only run once per VTK
//! version and set of build options.  `RVTK_VTK_SOURCE_DIR` can point at an
//! existing VTK source tree (still built by this script) for offline builds.
//!
//! Building VTK is by far the most expensive part (~8 minutes here).  To skip it
//! entirely, `RVTK_VTK_PREBUILT_DIR` can point at an already installed VTK (the
//! prefix produced by an earlier run, e.g. copied out of `target/`); the script
//! then only builds the shim against it.  This is meant for a shared or CI
//! cache, not for shipping: a prebuilt must be the exact version this crate was
//! generated against and, to keep the static guarantee, static.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// The VTK release the committed bindings are generated from.
const VTK_VERSION: &str = "9.7.1";
/// Official release tarball.  `RVTK_VTK_URL` overrides it.
const VTK_URL: &str = "https://www.vtk.org/files/release/9.7/VTK-9.7.1.tar.gz";
/// SHA-256 of `VTK_URL`.
const VTK_SHA256: &str = "cae04fd355004cb916a409db79d53a208f1221e975aeacc7540ee67b148ee91a";

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

fn main() -> Result<()> {
    println!("cargo:rerun-if-changed=shim");
    for variable in [
        "RVTK_VTK_SOURCE_DIR",
        "RVTK_VTK_URL",
        "RVTK_VTK_PREBUILT_DIR",
        "RVTK_CACHE_DIR",
        "CMAKE_GENERATOR",
        "DOCS_RS",
    ] {
        println!("cargo:rerun-if-env-changed={variable}");
    }

    // docs.rs cannot afford a VTK build; only the Rust code needs to compile.
    if env::var_os("DOCS_RS").is_some() {
        return Ok(());
    }

    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("cargo sets OUT_DIR"));
    let cache = cache_root(&out_dir).join(format!("vtk-{VTK_VERSION}"));

    // Prefer an explicit prebuilt install tree; otherwise build VTK from source.
    let modules = enabled_modules()?;
    let vtk = match prebuilt_vtk()? {
        Some(vtk) => vtk,
        None => {
            let source = vtk_source(&cache)?;
            build_vtk(&source, &cache, &modules)?
        }
    };
    let manifest = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("cargo sets it"));
    let shim = build_shim(&cache, &vtk, &manifest, &modules)?;
    link(&shim)
}

/// A VTK to link the shim against, plus a fingerprint identifying the exact
/// build so the shim cache can be keyed on it.
struct Vtk {
    /// Install prefix (contains `lib/cmake/vtk-*`, `include/`, `lib/`).
    prefix: PathBuf,
    /// Changes whenever a different VTK is used, invalidating the shim cache.
    fingerprint: String,
}

// ---------------------------------------------------------------------------
// cache location
// ---------------------------------------------------------------------------

/// Root of the (shared, persistent) VTK build cache, next to the cargo `target`
/// directory.  `RVTK_CACHE_DIR` overrides it, which lets several checkouts (or
/// CI jobs) share one VTK build.
fn cache_root(out_dir: &Path) -> PathBuf {
    if let Some(dir) = env::var_os("RVTK_CACHE_DIR") {
        return PathBuf::from(dir);
    }
    out_dir
        .ancestors()
        .find(|dir| dir.file_name().is_some_and(|name| name == "target"))
        .map_or_else(
            || out_dir.join("rvtk-cache"),
            |target| target.join("rvtk-vtk"),
        )
}

// ---------------------------------------------------------------------------
// VTK source
// ---------------------------------------------------------------------------

/// Fetch (or reuse) an unpacked VTK source tree.
fn vtk_source(cache: &Path) -> Result<PathBuf> {
    if let Some(dir) = env::var_os("RVTK_VTK_SOURCE_DIR") {
        let dir = PathBuf::from(dir);
        check_source(&dir)?;
        return Ok(dir);
    }

    fs::create_dir_all(cache)?;
    let source = cache.join(format!("VTK-{VTK_VERSION}"));
    let unpacked = cache.join(".unpacked");
    if unpacked.is_file() && source.join("CMakeLists.txt").is_file() {
        return Ok(source);
    }

    let tarball = cache.join(format!("VTK-{VTK_VERSION}.tar.gz"));
    if !tarball.is_file() {
        download(&tarball)?;
    }
    match sha256(&tarball) {
        Ok(digest) if digest.eq_ignore_ascii_case(VTK_SHA256) => {}
        Ok(digest) => {
            return Err(format!(
                "{} has SHA-256 {digest}, expected {VTK_SHA256}.\n\
                 Remove it to download a fresh copy, or set `RVTK_VTK_URL`.",
                tarball.display()
            )
            .into())
        }
        // No hashing tool available: trust the download (and say so).
        Err(error) => println!("cargo:warning=could not verify VTK archive: {error}"),
    }

    // A previous run may have died half way through unpacking.
    let _ = fs::remove_dir_all(&source);
    run(Command::new("cmake")
        .current_dir(cache)
        .args(["-E", "tar", "xzf"])
        .arg(&tarball))?;
    check_source(&source)?;
    fs::write(&unpacked, VTK_SHA256)?;
    Ok(source)
}

fn check_source(dir: &Path) -> Result<()> {
    let version = dir.join("CMake/vtkVersion.cmake");
    if dir.join("CMakeLists.txt").is_file() && version.is_file() {
        Ok(())
    } else {
        Err(format!("{} does not look like a VTK source tree", dir.display()).into())
    }
}

fn download(dest: &Path) -> Result<()> {
    let url = env::var("RVTK_VTK_URL").unwrap_or_else(|_| VTK_URL.to_owned());
    let partial = dest.with_extension("part");
    println!("cargo:warning=downloading VTK {VTK_VERSION} from {url}");

    let curl = Command::new("curl")
        .args(["-fL", "--retry", "3", "-o"])
        .arg(&partial)
        .arg(&url)
        .status();
    let ok = match curl {
        Ok(status) if status.success() => true,
        _ => Command::new("wget")
            .arg("-O")
            .arg(&partial)
            .arg(&url)
            .status()
            .is_ok_and(|status| status.success()),
    };
    if !ok {
        return Err(format!(
            "could not download {url}.\n\
             Install `curl` (or `wget`), download the tarball yourself and set\n\
             `RVTK_VTK_SOURCE_DIR` to the unpacked tree."
        )
        .into());
    }
    fs::rename(&partial, dest)?;
    Ok(())
}

/// SHA-256 of `path`, using whichever hashing tool is available.
fn sha256(path: &Path) -> Result<String> {
    let commands: [&[&str]; 3] = [
        &["shasum", "-a", "256"],
        &["sha256sum"],
        &["cmake", "-E", "sha256sum"],
    ];
    for command in commands {
        let Ok(output) = Command::new(command[0]).args(&command[1..]).arg(path).output() else {
            continue;
        };
        if !output.status.success() {
            continue;
        }
        let text = String::from_utf8_lossy(&output.stdout);
        if let Some(digest) = text.split_whitespace().next() {
            if digest.len() == 64 && digest.chars().all(|c| c.is_ascii_hexdigit()) {
                return Ok(digest.to_owned());
            }
        }
    }
    Err("no SHA-256 tool found (tried shasum, sha256sum, cmake)".into())
}

// ---------------------------------------------------------------------------
// VTK build
// ---------------------------------------------------------------------------

/// Build and install VTK as static libraries.  Returns the install prefix.
fn build_vtk(source: &Path, cache: &Path, modules: &[String]) -> Result<Vtk> {
    let out = cache.join("build");
    let stamp = cache.join(".vtk-built");
    let flags = vtk_flags(modules);
    let fingerprint = format!("{VTK_VERSION}\n{}\n", flags.join("\n"));
    if fs::read_to_string(&stamp).is_ok_and(|contents| contents == fingerprint)
        && vtk_cmake_dir(&out).is_ok()
    {
        return Ok(Vtk {
            prefix: out,
            fingerprint,
        });
    }

    let mut cmake = cmake::Config::new(source);
    // `out` is both the build tree root and the install prefix.
    cmake.out_dir(&out);
    for flag in &flags {
        let (name, value) = flag.split_once('=').expect("flags contain `=`");
        cmake.define(name, value);
    }
    cmake.build();

    fs::write(&stamp, &fingerprint)?;
    vtk_cmake_dir(&out)?;
    Ok(Vtk {
        prefix: out,
        fingerprint,
    })
}

// ---------------------------------------------------------------------------
// prebuilt VTK
// ---------------------------------------------------------------------------

/// An installed VTK from `RVTK_VTK_PREBUILT_DIR`, if that variable is set.
///
/// The warm-cache path already avoids rebuilding VTK, so this exists to skip the
/// build on a *cold* cache: point it at an install tree produced earlier (for
/// example one unpacked from a CI artifact) and only the shim is compiled.
fn prebuilt_vtk() -> Result<Option<Vtk>> {
    let Some(dir) = env::var_os("RVTK_VTK_PREBUILT_DIR") else {
        return Ok(None);
    };
    let prefix = PathBuf::from(dir);
    check_prebuilt(&prefix)?;
    println!(
        "cargo:warning=using prebuilt VTK at {} (skipping the VTK build)",
        prefix.display()
    );
    let fingerprint = format!("prebuilt\n{}\n", prefix.display());
    Ok(Some(Vtk { prefix, fingerprint }))
}

/// Validate a prebuilt install tree: it must be a VTK of the exact version the
/// committed bindings target, otherwise the shim would not match them.
fn check_prebuilt(prefix: &Path) -> Result<()> {
    let cmake_dir = vtk_cmake_dir(prefix).map_err(|error| {
        format!(
            "RVTK_VTK_PREBUILT_DIR={} is not an installed VTK: {error}",
            prefix.display()
        )
    })?;

    let version_file = cmake_dir.join("vtk-config-version.cmake");
    let text = fs::read_to_string(&version_file)
        .map_err(|error| format!("reading {}: {error}", version_file.display()))?;
    let found = parse_package_version(&text);
    if found.as_deref() != Some(VTK_VERSION) {
        return Err(format!(
            "RVTK_VTK_PREBUILT_DIR={} is VTK {}, but the bindings were generated \
             against VTK {VTK_VERSION}.",
            prefix.display(),
            found.as_deref().unwrap_or("unknown")
        )
        .into());
    }
    Ok(())
}

/// Pull the version out of a `set(PACKAGE_VERSION "9.7.1")` line.
fn parse_package_version(cmake: &str) -> Option<String> {
    for line in cmake.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("set(PACKAGE_VERSION") {
            let value = rest.trim().trim_end_matches(')').trim();
            let value = value.trim_matches('"').trim();
            if !value.is_empty() {
                return Some(value.to_owned());
            }
        }
    }
    None
}

/// CMake options: a static, minimal, wrapper-free VTK.
fn vtk_flags(modules: &[String]) -> Vec<String> {
    let mut flags: Vec<String> = [
        "CMAKE_BUILD_TYPE=Release",
        "BUILD_SHARED_LIBS=OFF",
        "CMAKE_POSITION_INDEPENDENT_CODE=ON",
        "VTK_BUILD_ALL_MODULES=OFF",
        "VTK_BUILD_TESTING=OFF",
        "VTK_BUILD_EXAMPLES=OFF",
        "VTK_USE_MPI=OFF",
        "VTK_SMP_IMPLEMENTATION_TYPE=Sequential",
        "VTK_WRAP_PYTHON=OFF",
        "VTK_WRAP_JAVA=OFF",
        "VTK_WRAP_JAVASCRIPT=OFF",
        "VTK_WRAP_SERIALIZATION=OFF",
    ]
    .iter()
    .map(|flag| (*flag).to_owned())
    .collect();
    flags.extend(
        modules
            .iter()
            .map(|module| format!("VTK_MODULE_ENABLE_VTK_{}=YES", module.trim_start_matches("vtk"))),
    );
    flags
}

/// The `lib/cmake/vtk-X.Y` directory of an installed VTK.
fn vtk_cmake_dir(prefix: &Path) -> Result<PathBuf> {
    let base = prefix.join("lib/cmake");
    let entries = fs::read_dir(&base)
        .map_err(|error| format!("reading {}: {error}", base.display()))?;
    let mut candidates: Vec<PathBuf> = entries
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("vtk-"))
        })
        .collect();
    candidates.sort();
    candidates
        .into_iter()
        .rev()
        .find(|path| path.join("vtk-config.cmake").is_file())
        .ok_or_else(|| format!("no VTK CMake package under {}", base.display()).into())
}

// ---------------------------------------------------------------------------
// shim build
// ---------------------------------------------------------------------------

/// The VTK modules to build, from the enabled Cargo features.
///
/// Every wrapped module is a Cargo feature named after it (`vtkFiltersCore`),
/// so a default build compiles a handful of modules instead of all of VTK.
fn enabled_modules() -> Result<Vec<String>> {
    let all = all_wrapped_modules()?;
    let enabled: Vec<String> = all
        .iter()
        .filter(|module| feature_enabled(module))
        .cloned()
        .collect();
    if enabled.is_empty() {
        return Err(format!(
            "no VTK modules enabled: turn on at least one of the `vtk*` Cargo \
             features ({} are available)",
            all.len()
        )
        .into());
    }
    Ok(enabled)
}

/// Whether the Cargo feature named after `module` is enabled.  Cargo exposes
/// every activated feature as `CARGO_FEATURE_<NAME>`, upper-cased.
fn feature_enabled(module: &str) -> bool {
    let variable = format!("CARGO_FEATURE_{}", module.to_uppercase().replace('-', "_"));
    env::var_os(variable).is_some()
}

/// Every wrapped VTK module, as generated into the shim's `CMakeLists.txt`
/// (`set(RVTK_ALL_MODULES ...)`).  Reading it there keeps this list and the
/// generated sources from drifting apart.
fn all_wrapped_modules() -> Result<Vec<String>> {
    let manifest = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("cargo sets it"));
    let cmake = fs::read_to_string(manifest.join("shim/CMakeLists.txt"))?;
    let after = cmake
        .split_once("set(RVTK_ALL_MODULES")
        .map(|(_, rest)| rest)
        .ok_or("shim/CMakeLists.txt has no `set(RVTK_ALL_MODULES ...)` block")?;
    let list = after
        .split_once(')')
        .map(|(list, _)| list)
        .ok_or("unterminated `set(RVTK_ALL_MODULES ...)` block")?;
    let modules: Vec<String> = list
        .split_whitespace()
        .filter(|word| is_module_name(word))
        .map(str::to_owned)
        .collect();
    if modules.is_empty() {
        return Err("no VTK modules found in shim/CMakeLists.txt".into());
    }
    Ok(modules)
}

fn is_module_name(word: &str) -> bool {
    word.len() > 3
        && word.starts_with("vtk")
        && word[3..].chars().all(|c| c.is_ascii_alphanumeric())
}

/// Configure and build the shim against the static VTK, installing
/// `librvtk_shim.a` and the link probe into a persistent cache directory, and
/// return that directory.
///
/// The build tree deliberately does *not* live in Cargo's `OUT_DIR`.  Cargo
/// keys `OUT_DIR` on the build script's fingerprint, so anything that makes the
/// script re-run (a profile change, a rebuilt build dependency, ...) would send
/// it to a brand new, empty directory and CMake would recompile all ~1500 shim
/// translation units from scratch.  A stable directory next to the VTK cache
/// lets CMake's own incremental build do the work: re-running the script with
/// unchanged sources is a quick no-op instead of a multi-minute rebuild.
fn build_shim(cache: &Path, vtk: &Vtk, manifest: &Path, modules: &[String]) -> Result<PathBuf> {
    let out = cache.join("shim");
    // The configuration fingerprint decides whether an existing build tree can
    // be reused at all; the content hash decides whether it is up to date.  The
    // enabled modules are part of it, so switching features rebuilds the shim.
    let config = format!(
        "{VTK_VERSION}\n{}\n{}\n{}\n{}\n",
        vtk.fingerprint,
        modules.join(","),
        env::var("TARGET").unwrap_or_default(),
        env::var("CMAKE_GENERATOR").unwrap_or_default(),
    );
    let content = format!("{:016x}\n", hash_dir(&manifest.join("shim"))?);

    let config_file = out.join(".shim-config");
    let content_file = out.join(".shim-hash");
    let installed = out.join("lib/librvtk_shim.a");
    let link_txt = out.join("build/CMakeFiles/rvtk_link_probe.dir/link.txt");
    let config_ok = fs::read_to_string(&config_file).is_ok_and(|saved| saved == config);
    let content_ok = fs::read_to_string(&content_file).is_ok_and(|saved| saved == content);
    if config_ok && content_ok && installed.is_file() && link_txt.is_file() {
        return Ok(out);
    }

    // A tree configured for another VTK/target/generator cannot be reused, so
    // start clean.  A mere source change keeps the tree and lets CMake rebuild
    // only the affected objects.
    if !config_ok {
        let _ = fs::remove_dir_all(&out);
    }

    let mut cmake = cmake::Config::new(manifest.join("shim"));
    cmake.out_dir(&out);
    cmake.define("CMAKE_BUILD_TYPE", "Release");
    cmake.define("RVTK_MODULES", modules.join(";"));
    cmake.define("VTK_DIR", vtk_cmake_dir(&vtk.prefix)?);
    cmake.define("CMAKE_PREFIX_PATH", &vtk.prefix);
    cmake.build();

    fs::write(&config_file, &config)?;
    fs::write(&content_file, &content)?;
    Ok(out)
}

/// A stable hash of every source file under `dir`, skipping build directories.
/// Two trees with the same contents hash the same even if their timestamps
/// differ, which is what lets an unchanged shim skip recompilation.
fn hash_dir(dir: &Path) -> Result<u64> {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut files = Vec::new();
    collect_files(dir, &mut files)?;
    files.sort();

    let mut hasher = DefaultHasher::new();
    for file in files {
        file.strip_prefix(dir)
            .expect("collected paths stay under the shim directory")
            .hash(&mut hasher);
        let contents = fs::read(&file)?;
        contents.len().hash(&mut hasher);
        contents.hash(&mut hasher);
    }
    Ok(hasher.finish())
}

fn collect_files(dir: &Path, files: &mut Vec<PathBuf>) -> Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if entry.file_type()?.is_dir() {
            // Build trees must never influence the hash.
            if path.file_name().is_some_and(|name| name == "build") {
                continue;
            }
            collect_files(&path, files)?;
        } else {
            files.push(path);
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// linking
// ---------------------------------------------------------------------------

/// Emit the `cargo:` directives that make rustc link the static shim and the
/// static VTK it contains.
///
/// The list of libraries is taken from the link line CMake produces for
/// `rvtk_link_probe`, the throw-away executable that links the shim: static VTK
/// pulls in dozens of archives plus the system frameworks they need, and CMake
/// already knows exactly which ones, in the right order.
fn link(shim: &Path) -> Result<()> {
    let lib_dir = shim.join("lib");
    println!("cargo:rustc-link-search=native={}", lib_dir.display());

    let link_txt = shim.join("build/CMakeFiles/rvtk_link_probe.dir/link.txt");
    let line = fs::read_to_string(&link_txt).map_err(|error| {
        format!(
            "reading {}: {error}\n\
             `rvtk-sys/shim/CMakeLists.txt` must build an executable called\n\
             `rvtk_link_probe`, and its link line is read from CMake's\n\
             `link.txt`, which only the Makefile generators produce.  Set\n\
             `CMAKE_GENERATOR` to `Unix Makefiles`, `MinGW Makefiles` or\n\
             `NMake Makefiles` (Ninja and Visual Studio do not write one).",
            link_txt.display()
        )
    })?;
    let parsed = parse_link_line(&split_args(&line));

    let mut search_dirs = vec![lib_dir.display().to_string()];
    for dir in parsed.search_dirs {
        if !search_dirs.contains(&dir) {
            search_dirs.push(dir);
        }
    }
    for dir in search_dirs {
        println!("cargo:rustc-link-search=native={dir}");
    }
    for library in parsed.libraries {
        println!("cargo:rustc-link-lib={library}");
    }

    // The link line above goes through a C++ driver, which adds the C++ runtime
    // implicitly; rustc links with `cc`, so ask for it explicitly.  MSVC needs
    // nothing here: its object files carry `#pragma comment(lib, ...)`
    // directives that the linker honours on its own.
    let target = env::var("TARGET").unwrap_or_default();
    if target.contains("apple") {
        println!("cargo:rustc-link-lib=dylib=c++");
    } else if target.contains("linux")
        || target.contains("freebsd")
        || target.contains("windows-gnu")
    {
        println!("cargo:rustc-link-lib=dylib=stdc++");
    }
    Ok(())
}

#[derive(Default)]
struct LinkLine {
    search_dirs: Vec<String>,
    /// Entries as rustc wants them: `static=name`, `dylib=name`,
    /// `framework=name`.
    libraries: Vec<String>,
}

fn parse_link_line(args: &[String]) -> LinkLine {
    let mut parsed = LinkLine::default();
    // The first argument is the compiler driver.
    for (index, arg) in args.iter().enumerate().skip(1) {
        let arg = arg.as_str();
        // `-framework Foo`
        if arg == "-framework" {
            if let Some(name) = args.get(index + 1) {
                parsed.libraries.push(format!("framework={name}"));
            }
        } else if let Some(rest) = arg.strip_prefix("-Wl,") {
            // `-Wl,-force_load,<archive>` means whole-archive, which rustc
            // spells `static:+whole-archive=<name>`.  Every other `-Wl,` flag
            // (rpath, search paths, ...) is irrelevant for a static link.
            let parts: Vec<&str> = rest.split(',').collect();
            let whole_archive = parts
                .iter()
                .position(|part| *part == "-force_load" || *part == "-force_load_swift_libs")
                .and_then(|position| parts.get(position + 1));
            if let Some(path) = whole_archive {
                push_archive(&mut parsed, path, true);
            }
        } else if let Some(dir) = arg.strip_prefix("-L") {
            if !dir.is_empty() {
                parsed.search_dirs.push(dir.to_owned());
            }
        } else if let Some(name) = arg.strip_prefix("-l") {
            if !name.is_empty() {
                parsed.libraries.push(format!("dylib={name}"));
            }
        } else if !arg.starts_with('-') && archive_name(arg).is_some() {
            push_archive(&mut parsed, arg, false);
        }
    }
    parsed
}

fn push_archive(parsed: &mut LinkLine, path: &str, whole_archive: bool) {
    let Some((dir, name)) = archive_name(path) else {
        return;
    };
    if !dir.is_empty() {
        parsed.search_dirs.push(dir);
    }
    if whole_archive {
        parsed
            .libraries
            .push(format!("static:+whole-archive={name}"));
    } else {
        parsed.libraries.push(format!("static={name}"));
    }
}

/// Split an archive path into its directory and the name rustc needs
/// (`libvtkCommonCore-9.7.1.a` -> `vtkCommonCore-9.7.1`).
fn archive_name(path: &str) -> Option<(String, String)> {
    let name = path.strip_suffix(".a")?;
    let (dir, file) = match name.rfind('/') {
        Some(slash) => (&name[..slash], &name[slash + 1..]),
        None => ("", name),
    };
    let file = file.strip_prefix("lib").unwrap_or(file);
    if file.is_empty() {
        return None;
    }
    Some((dir.to_owned(), file.to_owned()))
}

/// Split a command line into arguments, honouring quotes and backslashes.
fn split_args(line: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    let mut started = false;
    let mut chars = line.chars();
    while let Some(character) = chars.next() {
        match character {
            '\\' => {
                if let Some(escaped) = chars.next() {
                    current.push(escaped);
                    started = true;
                }
            }
            '"' => {
                quoted = !quoted;
                started = true;
            }
            character if character.is_whitespace() && !quoted => {
                if started {
                    args.push(std::mem::take(&mut current));
                    started = false;
                }
            }
            character => {
                current.push(character);
                started = true;
            }
        }
    }
    if started {
        args.push(current);
    }
    args
}

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

fn run(command: &mut Command) -> Result<()> {
    let status = command
        .status()
        .map_err(|error| format!("running {command:?}: {error}"))?;
    if !status.success() {
        return Err(format!("{command:?} failed with {status}").into());
    }
    Ok(())
}
