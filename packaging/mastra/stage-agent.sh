#!/usr/bin/env bash
# Put the built-in agent's folder into a package tree, after checking it.
#
# The folder (`mastra-agent`: a Node program, the agent's entry files, Mastra Code's SDK without
# Mastra's enterprise-licensed content) is built beforehand by
#   node packages/mastra-runtime/scripts/agent-package.mjs build --target <triple> --out <dir>
# This script runs that script's `check` on it for the same target and copies it only when the
# check passes: whole, for this target, no enterprise content, no refused or unstated licence,
# nothing that looks like a secret or a stored sign-in. On a failure nothing is copied.
#
# This is not the admission gate of the Mastra workflow runtime (stage.sh, `mastra/`): the agent's
# folder is for test packages, goes through no approval and adds none. See docs/mastra-packaging.md.
#
# Usage:
#   packaging/mastra/stage-agent.sh --target <triple> --dest <dir> [--from <built folder>]
#
# Environment:
#   SOLIDCRAFT_MASTRA_AGENT_DIR   the built folder (default: $CARGO_TARGET_DIR/mastra-agent/<triple>)
#   SOLIDCRAFT_MASTRA_STAGE_TEST  TEST ONLY: allows --pin and --skills-root, a folder checked
#                                 against in place of packages/mastra-runtime/agent-package and of
#                                 the repository's skills
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

TARGET=
DEST=
FROM="${SOLIDCRAFT_MASTRA_AGENT_DIR:-}"
PIN=
SKILLS_ROOT=
while [ $# -gt 0 ]; do
  case "$1" in
    --target) TARGET="${2:-}"; shift 2 ;;
    --dest) DEST="${2:-}"; shift 2 ;;
    --from) FROM="${2:-}"; shift 2 ;;
    --pin) PIN="${2:-}"; shift 2 ;;
    --skills-root) SKILLS_ROOT="${2:-}"; shift 2 ;;
    -h | --help) sed -n '2,27p' "$0"; exit 0 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done

die() {
  echo "error: $*" >&2
  exit 1
}

if [ -n "$PIN" ] && [ "${SOLIDCRAFT_MASTRA_STAGE_TEST:-0}" != 1 ]; then
  die "--pin is test-only; packaging always checks against packages/mastra-runtime/agent-package"
fi
if [ -n "$SKILLS_ROOT" ] && [ "${SOLIDCRAFT_MASTRA_STAGE_TEST:-0}" != 1 ]; then
  die "--skills-root is test-only; packaging always checks the skills against the repository"
fi
[ -n "$TARGET" ] || die "--target is required"
[ -n "$DEST" ] || die "--dest is required"
case "$DEST" in
  / | /*/.. | */../* | *'/..') die "--dest must be a concrete directory, not $DEST" ;;
esac
FROM="${FROM:-${CARGO_TARGET_DIR:-$ROOT/target}/mastra-agent/$TARGET}"
[ -d "$FROM" ] || die "no built agent folder at $FROM; build it first: node packages/mastra-runtime/scripts/agent-package.mjs build --target $TARGET --out $FROM"
command -v node >/dev/null || die "node is needed to check the agent folder"

echo "==> checking the built-in agent's folder ($TARGET)"
check=(node "$ROOT/packages/mastra-runtime/scripts/agent-package.mjs" check --target "$TARGET")
if [ -n "$PIN" ]; then check+=(--pin "$PIN"); fi
if [ -n "$SKILLS_ROOT" ]; then check+=(--skills-root "$SKILLS_ROOT"); fi
"${check[@]}" "$FROM" || die "the built-in agent's folder did not pass its check; nothing was staged"

echo "==> staging the built-in agent in $DEST"
mkdir -p "$(dirname "$DEST")"
# A folder staged earlier may be one nothing may write.
if [ -d "$DEST" ]; then chmod -R u+w "$DEST"; fi
rm -rf "$DEST"
cp -R "$FROM" "$DEST"
echo "ok built-in agent staged: $(du -sh "$DEST" | cut -f1)"
