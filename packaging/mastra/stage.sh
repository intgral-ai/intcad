#!/usr/bin/env bash
# Stage the approved Mastra runtime into a package tree, or fail closed.
#
# The runtime artifact is currently UNAPPROVED (#15): four shipped @mastra/core/dist/ee-* files
# carry the Mastra Enterprise License. This script therefore requires the artifact admission gate
# to pass before it copies anything. Until a reviewed policy approval exists, packaging stops with
# `UNAPPROVED_ARTIFACT`, which is the intended legal gate rather than a build failure to work
# around. It never signs or publishes anything.
#
# Usage:
#   packaging/mastra/stage.sh --target <triple> --dest <dir> [--artifact <dir>] [--inputs <dir>]
#                             [--approval <id>]
#
# Environment:
#   SOLIDCRAFT_MASTRA_ARTIFACT          artifact tree (default: target/mastra-runtime-artifact)
#   SOLIDCRAFT_MASTRA_INPUTS            input tree   (default: target/mastra-runtime-inputs)
#   SOLIDCRAFT_MASTRA_APPROVAL          exact approval id from packaging/mastra/policy.json
#   SOLIDCRAFT_RUNTIME_ARTIFACT_CHECK   test seam: one executable replacing `cargo xtask runtime-artifact`
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

TARGET=
DEST=
ARTIFACT="${SOLIDCRAFT_MASTRA_ARTIFACT:-$ROOT/target/mastra-runtime-artifact}"
INPUTS="${SOLIDCRAFT_MASTRA_INPUTS:-$ROOT/target/mastra-runtime-inputs}"
APPROVAL="${SOLIDCRAFT_MASTRA_APPROVAL:-}"

while [ $# -gt 0 ]; do
  case "$1" in
    --target) TARGET="${2:-}"; shift 2 ;;
    --dest) DEST="${2:-}"; shift 2 ;;
    --artifact) ARTIFACT="${2:-}"; shift 2 ;;
    --inputs) INPUTS="${2:-}"; shift 2 ;;
    --approval) APPROVAL="${2:-}"; shift 2 ;;
    -h | --help) sed -n '2,22p' "$0"; exit 0 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done

die() {
  echo "error: $*" >&2
  exit 1
}

[ -n "$TARGET" ] || die "--target is required"
[ -n "$DEST" ] || die "--dest is required"
case "$DEST" in
  / | /*/.. | */../* | *'/..') die "--dest must be a concrete directory, not $DEST" ;;
esac

# The 1.0 feature matrix: Windows x86_64 MSVC, macOS aarch64/x86_64 and Linux x86_64 glibc.
case "$TARGET" in
  x86_64-unknown-linux-gnu) DEFAULT_APPROVAL=mastra-runtime-linux-x64-v1 ;;
  x86_64-pc-windows-msvc) DEFAULT_APPROVAL=mastra-runtime-windows-x64-v1 ;;
  aarch64-apple-darwin) DEFAULT_APPROVAL=mastra-runtime-macos-arm64-v1 ;;
  x86_64-apple-darwin) DEFAULT_APPROVAL=mastra-runtime-macos-x64-v1 ;;
  *) die "Mastra 1.0 does not ship for $TARGET (supported: x86_64-unknown-linux-gnu, x86_64-pc-windows-msvc, aarch64-apple-darwin, x86_64-apple-darwin)" ;;
esac
APPROVAL="${APPROVAL:-$DEFAULT_APPROVAL}"

[ -f "$ARTIFACT/runtime.json" ] || die "no runtime artifact at $ARTIFACT; packaging fails closed until #15 records an approved artifact (set SOLIDCRAFT_MASTRA_ARTIFACT)"
[ -d "$INPUTS" ] || die "no artifact input tree at $INPUTS; set SOLIDCRAFT_MASTRA_INPUTS"

sha256_file() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | cut -d' ' -f1
  else
    shasum -a 256 "$1" | cut -d' ' -f1
  fi
}
digest() {
  local file="$1"
  case "$file" in
    /*) ;;
    *) file="$ROOT/$file" ;;
  esac
  printf 'sha256:%s' "$(sha256_file "$file")"
}

REVISION="$(git -C "$ROOT" rev-parse HEAD)"
CONTRACT="$(digest schema/mastra-runtime/v1/manifest.json)"
POLICY="$(digest packaging/mastra/policy.json)"
SOURCES="$(digest packaging/mastra/source-records.json)"
RUNTIME_VERSION="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["version"])' "$ROOT/packages/mastra-runtime/package.json")"
NODE_VERSION="$(tr -d '[:space:]' <"$ROOT/.nvmrc")"

echo "==> Mastra artifact admission gate ($TARGET, approval $APPROVAL)"
if [ -n "${SOLIDCRAFT_RUNTIME_ARTIFACT_CHECK:-}" ]; then
  if ! "$SOLIDCRAFT_RUNTIME_ARTIFACT_CHECK" \
    --mode artifact \
    --tree "$ARTIFACT" \
    --inputs-root "$INPUTS" \
    --approval "$APPROVAL" \
    --target "$TARGET" \
    --expected-revision "$REVISION" \
    --expected-contract-digest "$CONTRACT" \
    --expected-policy-digest "$POLICY" \
    --expected-sources-digest "$SOURCES" \
    --advertised true; then
    die "the Mastra runtime artifact was not admitted; it is unapproved or does not match this revision. No package was produced."
  fi
else
  if ! (cd "$ROOT" && cargo xtask runtime-artifact \
    --mode artifact \
    --tree "$ARTIFACT" \
    --inputs-root "$INPUTS" \
    --approval "$APPROVAL" \
    --target "$TARGET" \
    --expected-revision "$REVISION" \
    --expected-contract-digest "$CONTRACT" \
    --expected-policy-digest "$POLICY" \
    --expected-sources-digest "$SOURCES" \
    --advertised true); then
    die "the Mastra runtime artifact was not admitted; it is unapproved or does not match this revision. No package was produced."
  fi
fi

python3 "$ROOT/packaging/mastra/validate_runtime.py" \
  --runtime "$ARTIFACT" \
  --target "$TARGET" \
  --revision "$REVISION" \
  --contract-digest "$CONTRACT" \
  --policy-digest "$POLICY" \
  --sources-digest "$SOURCES" \
  --runtime-version "$RUNTIME_VERSION" \
  --node-version "$NODE_VERSION"

echo "==> staging Mastra runtime in $DEST"
mkdir -p "$(dirname "$DEST")"
rm -rf "$DEST"
cp -R "$ARTIFACT" "$DEST"
case "$TARGET" in
  x86_64-unknown-linux-gnu | *-apple-darwin) chmod 755 "$DEST/node" ;;
esac
echo "ok Mastra runtime staged: $(du -sh "$DEST" | cut -f1)"
