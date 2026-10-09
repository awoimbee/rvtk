#!/usr/bin/env bash
#
# Hunt down every call in the generated smoke test that segfaults.
#
#   ./scripts/crash-hunt.sh [output-file]
#
# The smoke test prints the name of each check before running it and honours
# the VTK_WRAP_SKIP environment variable.  This script runs it repeatedly: whenever
# the process dies it records the last check that was printed, adds it to the
# skip list and starts again.  The result is the complete list of crashing
# calls, each of which is then reproduced in plain C++ to decide whether the
# fault is in vtk-wrap or in VTK itself.
#
# Requires `cargo build -p vtk-wrap --test smoke` to have been run first.

set -uo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT="${1:-$REPO_ROOT/target/crash-hunt.txt}"
LOG="${LOG:-$REPO_ROOT/target/crash-hunt.log}"
MAX_ROUNDS="${MAX_ROUNDS:-2000}"

BIN="$(find "$REPO_ROOT/target/debug/deps" -maxdepth 1 -type f -perm +111 -name 'smoke-*' | head -1)"
if [[ -z "$BIN" ]]; then
  echo "error: build the smoke test first: cargo build -p vtk-wrap --test smoke" >&2
  exit 1
fi

: > "$OUT"
SKIP=""
for ((round = 1; round <= MAX_ROUNDS; round++)); do
  VTK_WRAP_SKIP="$SKIP" "$BIN" >"$LOG" 2>&1
  status=$?
  if [[ $status -eq 0 ]]; then
    echo "clean after $round runs; $(wc -l < "$OUT" | tr -d ' ') crashing calls recorded in $OUT"
    exit 0
  fi

  # The last check that got as far as printing its name is the one that died.
  name="$(grep -E '^  \S+$' "$LOG" | tail -1 | sed 's/^  //')"
  if [[ -z "$name" ]]; then
    echo "error: run $round died (status $status) without printing a check name" >&2
    tail -5 "$LOG" >&2
    exit 1
  fi
  if grep -qxF "$name" "$OUT"; then
    echo "error: '$name' crashed twice; cannot make progress" >&2
    exit 1
  fi

  printf '%s\n' "$name" >> "$OUT"
  SKIP="${SKIP:+$SKIP,}$name"
  if (( round % 25 == 0 )); then
    echo "  ... $round crashes so far (latest: $name)"
  fi
done

echo "error: gave up after $MAX_ROUNDS rounds" >&2
exit 1
