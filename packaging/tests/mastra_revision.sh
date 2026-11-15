#!/usr/bin/env bash
# A source tree without .git must still compile: the host carries the unbound revision sentinel and
# the packaged-runtime gate refuses it. A strict release/Mastra build without a real revision must
# fail instead of producing an unbindable package.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

run_check() {
  # GIT_DIR points nowhere, so both the build script's `git rev-parse` and any accidental lookup
  # outside the checkout see no repository.
  env -u SOLIDCRAFT_GIT_REVISION_OVERRIDE \
    GIT_DIR=/nonexistent \
    SOLIDCRAFT_RELEASE=0 \
    SOLIDCRAFT_WITH_MASTRA="$1" \
    cargo check -q -p solidcraft-ui-egui --lib
}

if ! ordinary="$(run_check 0 2>&1)"; then
  printf '%s\n' "$ordinary" >&2
  echo "error: an ordinary source build without git failed; the unbound sentinel must compile" >&2
  exit 1
fi
if strict="$(run_check 1 2>&1)"; then
  echo "error: a strict Mastra build without a real revision unexpectedly succeeded" >&2
  exit 1
fi
case "$strict" in
  *"cannot determine the full 40-character Git revision"*) ;;
  *)
    printf '%s\n' "$strict" >&2
    echo "error: a strict build failed for an unexpected reason" >&2
    exit 1
    ;;
esac
echo "ok: source build compiled with the unbound sentinel; strict build refused it"
