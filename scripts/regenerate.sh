#!/usr/bin/env bash
#
# Regenerate the Rust and C++ bindings from the VTK headers.
#
#   ./scripts/regenerate.sh
#
# The script:
#   1. locates an installed VTK (override with VTK_DIR),
#   2. builds WrapVTK's `vtkWrapXML` tool,
#   3. dumps an XML description of every wrapped module,
#   4. runs `vtk-gen` to rewrite the C++ shim and the Rust crates.
#
# Set WRAPVTK_DIR to reuse an existing WrapVTK checkout, and MODULES to change
# the list of wrapped VTK modules (semicolon separated).

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BUILD_DIR="${BUILD_DIR:-$REPO_ROOT/target/wrapvtk}"
WRAPVTK_DIR="${WRAPVTK_DIR:-$BUILD_DIR/WrapVTK}"

# Never block on a git credential prompt.
export GIT_TERMINAL_PROMPT=0

# The VTK modules we generate bindings for.  Note that vtk-gen expects the
# module names VTK uses internally (no `vtk` prefix, semicolon separated).
MODULES="${MODULES:-vtkCommonCore;vtkCommonDataModel;vtkCommonExecutionModel;vtkCommonMath;vtkCommonTransforms;vtkCommonColor;vtkCommonSystem;vtkCommonMisc;vtkFiltersCore;vtkFiltersSources;vtkFiltersGeneral;vtkFiltersGeometry;vtkIOGeometry;vtkIOCore;vtkIOLegacy;vtkIOPLY;vtkImagingHybrid}"

# WrapVTK expects the CMake component names, i.e. without the `vtk` prefix.
WRAP_MODULES="$(echo "$MODULES" | sed 's/vtk\([A-Za-z0-9]*\)/\1/g')"

# --- locate VTK -------------------------------------------------------------
if [[ -z "${VTK_DIR:-}" ]]; then
  if command -v brew >/dev/null 2>&1 && brew --prefix vtk >/dev/null 2>&1; then
    VTK_PREFIX="$(brew --prefix vtk)"
  else
    VTK_PREFIX="/usr"
  fi
  VTK_DIR="$(ls -d "$VTK_PREFIX"/lib/cmake/vtk-* 2>/dev/null | sort -V | tail -1)"
fi
if [[ -z "${VTK_DIR:-}" || ! -d "$VTK_DIR" ]]; then
  echo "error: could not find VTK; set VTK_DIR to the directory containing vtk-config.cmake" >&2
  exit 1
fi
echo "Using VTK_DIR=$VTK_DIR"

# --- fetch WrapVTK ----------------------------------------------------------
if [[ ! -d "$WRAPVTK_DIR" ]]; then
  mkdir -p "$(dirname "$WRAPVTK_DIR")"
  git clone --depth 1 https://github.com/dgobbi/WrapVTK.git "$WRAPVTK_DIR"
fi
cp "$REPO_ROOT/tools/wrapvtk-CMakeLists.txt" "$WRAPVTK_DIR/CMakeLists.txt"

# --- build vtkWrapXML and generate the XML ---------------------------------
cmake -S "$WRAPVTK_DIR" -B "$WRAPVTK_DIR/build" \
  -DCMAKE_BUILD_TYPE=Release \
  -DVTK_DIR="$VTK_DIR" \
  -DWRAPVTK_MODULES="$WRAP_MODULES"
cmake --build "$WRAPVTK_DIR/build" --parallel

# --- run the generator ------------------------------------------------------
cargo run --release -p vtk-gen -- \
  --xml-dir "$WRAPVTK_DIR/build/xml" \
  --repo "$REPO_ROOT" \
  --modules "$(echo "$MODULES" | tr ';' ',')"

echo "Done. Rebuild with: cargo build -p vtk-sys -p vtk"
