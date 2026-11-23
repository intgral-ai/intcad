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

# The archive's own header ownership, before extraction loses it; the package's `.MTREE` names a
# different owner for its file list, and both are kept as they were.
zstd -dc "$PKG" > "$WORK/pkg.tar"
archive_owner="$(python3 -c 'import sys, tarfile; m = tarfile.open(sys.argv[1]).getmembers()[0]; print(m.uid, m.gid)' "$WORK/pkg.tar" 2>/dev/null || true)"
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
  # The package's own ownership, as makepkg wrote it into .MTREE's set line; pacman ignores it,
  # but the repack must not silently rewrite what the build recorded.
  uid="$(zcat .MTREE 2>/dev/null | sed -n 's#.*uid=\([0-9][0-9]*\).*#\1#p' | head -n 1)"
  gid="$(zcat .MTREE 2>/dev/null | sed -n 's#.*gid=\([0-9][0-9]*\).*#\1#p' | head -n 1)"
  rm -f .MTREE
  # Installed size counts each inode once (a hardlink is not a second copy).
  size="$(find . -type f ! -path './.PKGINFO' ! -path './.BUILDINFO' ! -path './.MTREE' \
    -printf '%i %s\n' | sort -u -k1,1 | awk '{ total += $2 } END { print total + 0 }')"
  sed -i "s/^size = .*/size = $size/" .PKGINFO
  owner=()
  [ -z "$uid" ] || owner+=(--uid "$uid")
  [ -z "$gid" ] || owner+=(--gid "$gid")
  find . -mindepth 1 -maxdepth 1 ! -name .MTREE -print0 | sort -z |
    bsdtar -czf .MTREE --format=mtree "${owner[@]}" \
      --options='!all,use-set,type,uid,gid,mode,time,size,md5,sha256,link' --null -T -
  # The package's top-level names as makepkg writes them (`usr/...`, not `./usr/...`), and every
  # top-level entry the package carried (`.PKGINFO`, `.BUILDINFO`, `.MTREE`, `.INSTALL`).
  # The archive keeps the ownership its headers had; the `.MTREE` keeps its own set line.
  header_owner=()
  if [ -n "${archive_owner:-}" ]; then
    read -r a_uid a_gid <<<"$archive_owner"
    header_owner=(--uid "$a_uid" --gid "$a_gid")
  fi
  find . -mindepth 1 -maxdepth 1 -printf '%P\0' | sort -z |
    bsdtar -cf - --null "${header_owner[@]}" -T - | zstd -c -T0 --ultra -20 - > "$WORK/package.pkg.tar.zst"
)

mv "$WORK/package.pkg.tar.zst" "$PKG"
echo "wrote $PKG with the built-in agent"
