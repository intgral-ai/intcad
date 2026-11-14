#!/usr/bin/env bash
# Add runtime dependencies to an already built Arch package's .PKGINFO.
#
# Usage: packaging/linux/arch-depends.sh PACKAGE.pkg.tar.zst DEPENDENCY...
#
# The arch-package action that builds our Arch package (storytold/craft-common-actions,
# pinned in release.yml) writes a fixed depends list that predates the Assistant's web view
# and has no input for extra dependencies. Rewriting .PKGINFO is the only place left: the
# package files themselves are unchanged, so .MTREE stays valid. .PKGINFO is written first,
# where pacman expects it. Calling it again is a no-op for dependencies already declared.
#
# Keep the dependency names to what pacman calls the packages (gtk3, webkit2gtk-4.1), not
# sonames: pacman cannot resolve a soname depend in an Arch package.
set -euo pipefail

pkg="${1:?usage: arch-depends.sh PACKAGE.pkg.tar.zst DEPENDENCY...}"
shift
[ $# -gt 0 ] || { echo "error: no dependencies given" >&2; exit 2; }
[ -f "$pkg" ] || { echo "error: $pkg not found" >&2; exit 1; }
for tool in tar zstd bsdtar gzip; do
  command -v "$tool" >/dev/null || { echo "error: $tool not found (Arch packaging needs libarchive-tools for .MTREE)" >&2; exit 1; }
done

work="$(mktemp -d "${TMPDIR:-/tmp}/arch-depends.XXXXXX")"
trap 'rm -rf -- "$work" "$work.pkg.tar.zst"' EXIT

tar --zstd -xf "$pkg" -C "$work"
info="$work/.PKGINFO"
[ -f "$info" ] || { echo "error: $pkg has no .PKGINFO" >&2; exit 1; }

changed=0
for dep in "$@"; do
  # A plain package name, so it cannot inject a PKGBUILD/vercmp expression or shell text.
  case "$dep" in
    '' | *[!A-Za-z0-9@._+-]*) echo "error: invalid Arch dependency '$dep'" >&2; exit 1 ;;
  esac
  if ! grep -qxF "depend = $dep" "$info"; then
    printf 'depend = %s\n' "$dep" >>"$info"
    changed=1
  fi
done
if [ "$changed" = 0 ]; then
  echo "$(basename "$pkg") already declares: $*"
  exit 0
fi

# `.MTREE` carries .PKGINFO's hash as well, so changing .PKGINFO without regenerating it leaves
# `pacman -Qkk` reporting a modified metadata file. Regenerate it with makepkg's own tool and
# options before repacking; a package without a valid .MTREE is not a package this script makes.
(
  cd "$work"
  rm -f .MTREE
  bsdtar -cf - --format=mtree --exclude '.MTREE' \
    --options='!all,use-set,type,uid,gid,mode,time,size,md5,sha256,link' . | gzip -n >.MTREE
)
gzip -dc "$work/.MTREE" | grep -q '\.PKGINFO' || {
  echo "error: regenerated .MTREE does not name .PKGINFO" >&2
  exit 1
}

# Metadata first, then every package entry in a stable order (directories before their
# contents). No recursion: the list already names every entry, so nothing is added twice.
# The repacked file is written beside the extraction directory, so find cannot see it.
out="${work}.pkg.tar.zst"
(
  cd "$work"
  {
    printf '%s\n' .PKGINFO
    for name in .BUILDINFO .MTREE .INSTALL; do
      [ -e "$name" ] && printf '%s\n' "$name"
    done
    find . -mindepth 1 ! -name .PKGINFO ! -name .BUILDINFO ! -name .MTREE ! -name .INSTALL \
      -printf '%P\n' | LC_ALL=C sort
  } | tar --zstd --no-recursion --numeric-owner --owner=0 --group=0 -cf "$out" -T -
)
mv "$out" "$pkg"
echo "$(basename "$pkg") now declares: $*"
