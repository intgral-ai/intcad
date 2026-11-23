#!/usr/bin/env bash
# Put the built-in Mastra agent into an Arch package.
#
#   packaging/linux/arch-agent.sh <package.pkg.tar.zst> <tarball.tar.gz>
#
# The `arch-package` action's PKGBUILD copies only the tarball's `bin/` and `share/` into the
# package, so `lib/intcad/` — where the agent lives (`usr/lib/intcad/mastra-agent`) — never
# reaches it. The release promises the agent in every Linux package it supports, so this copies
# that one folder from the same tarball the package was built from, regenerates the package's
# `.MTREE` file list the way makepkg does, and repacks it. Idempotent: a package that already
# carries the agent is left alone.
set -euo pipefail

PKG="${1:?usage: arch-agent.sh <package.pkg.tar.zst> <tarball.tar.gz>}"
TARBALL="${2:?usage: arch-agent.sh <package.pkg.tar.zst> <tarball.tar.gz>}"

for tool in bsdtar zstd tar; do
  command -v "$tool" >/dev/null || { echo "error: $tool not found (apt install libarchive-tools zstd)" >&2; exit 1; }
done
[ -f "$PKG" ] || { echo "error: $PKG not found" >&2; exit 1; }
[ -f "$TARBALL" ] || { echo "error: $TARBALL not found" >&2; exit 1; }

WORK="$(mktemp -d "${TMPDIR:-/tmp}/arch-agent.XXXXXX")"
trap 'rm -rf "$WORK"' EXIT
mkdir -p "$WORK/pkg" "$WORK/src"

bsdtar -xf "$PKG" -C "$WORK/pkg"
if [ -d "$WORK/pkg/usr/lib/intcad/mastra-agent" ]; then
  echo "the agent is already in $PKG"
  exit 0
fi

# The tarball wraps its tree in one folder and holds `bin/`, `lib/`, `share/`; the Arch package
# puts the first two under `usr/`. --strip-components=1 gives the tarball's own paths.
tar -xzf "$TARBALL" -C "$WORK/src" --strip-components=1 \
  --wildcards '*/lib/intcad/mastra-agent/*'
SOURCE="$WORK/src/lib/intcad/mastra-agent"
[ -f "$SOURCE/agent.json" ] || { echo "error: $TARBALL carries no lib/intcad/mastra-agent" >&2; exit 1; }
[ -f "$SOURCE/agent/main.mjs" ] || { echo "error: the agent folder has no agent/main.mjs" >&2; exit 1; }

install -d "$WORK/pkg/usr/lib/intcad"
cp -a "$SOURCE" "$WORK/pkg/usr/lib/intcad/mastra-agent"

# The package's own file list and installed size, regenerated the way makepkg writes them, so the
# entries it adds are part of the package's record rather than a stale file beside them.
(
  cd "$WORK/pkg"
  rm -f .MTREE
  size="$(find . -type f ! -name .PKGINFO ! -name .BUILDINFO ! -name .MTREE -printf '%s\n' |
    awk '{ total += $1 } END { print total + 0 }')"
  sed -i "s/^size = .*/size = $size/" .PKGINFO
  find . -mindepth 1 -maxdepth 1 ! -name .MTREE -print0 | sort -z |
    bsdtar -czf .MTREE --format=mtree \
      --options='!all,use-set,type,uid,gid,mode,time,size,md5,sha256,link' --null -T -
  # The package's top-level names as makepkg writes them (`usr/...`, not `./usr/...`), and every
  # top-level entry the package carried (`.PKGINFO`, `.BUILDINFO`, `.MTREE`, `.INSTALL`).
  find . -mindepth 1 -maxdepth 1 -printf '%P\0' | sort -z |
    bsdtar -cf - --null -T - | zstd -c -T0 --ultra -20 - > "$WORK/package.pkg.tar.zst"
)

mv "$WORK/package.pkg.tar.zst" "$PKG"
echo "wrote $PKG with the built-in agent"
