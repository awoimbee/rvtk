#!/usr/bin/env bash
#
# Regenerate the committed Rust and C++ bindings from VTK's headers.
#
#   ./scripts/regenerate.sh
#
# `vtk-wrap-gen` does the actual work: it clones (or reuses) WrapVTK, builds
# `vtkWrapXML` against your VTK, generates the XML API description, and then
# writes the C++ shim and the Rust crates.  This script is only a convenience
# wrapper that builds `vtk-wrap-gen` and forwards its environment.
#
# By default the module set comes from the `vtk*` features in
# `vtk-wrap-sys/Cargo.toml`, so the bindings and the features cannot drift apart.
# Override with MODULES (comma separated, VTK spelling):
#
#   MODULES=vtkCommonCore,vtkFiltersSources ./scripts/regenerate.sh
#
# Recognised environment variables (all optional):
#   VTK_DIR        VTK CMake package dir; auto-detected (Homebrew, /usr/local,
#                  /usr) when unset.
#   VTK_INCLUDE    VTK include dir; derived from VTK_DIR when unset.
#   WRAPVTK_DIR    WrapVTK checkout to use or create (default:
#                  target/wrapvtk/WrapVTK).
#   MODULES        Modules to wrap (default: the vtk-wrap-sys features).

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

args=(--repo "$REPO_ROOT")
[[ -n "${MODULES:-}" ]] && args+=(--modules "$MODULES")
[[ -n "${VTK_DIR:-}" ]] && args+=(--vtk-dir "$VTK_DIR")
[[ -n "${VTK_INCLUDE:-}" ]] && args+=(--vtk-include "$VTK_INCLUDE")
[[ -n "${WRAPVTK_DIR:-}" ]] && args+=(--wrapvtk-dir "$WRAPVTK_DIR")
[[ -n "${WRAPVTK_URL:-}" ]] && args+=(--wrapvtk-url "$WRAPVTK_URL")

cargo run --release -p vtk-wrap-gen -- "${args[@]}"

echo
echo "Done. Rebuild with: cargo build -p vtk-wrap-sys -p vtk-wrap"
