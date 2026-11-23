#!/usr/bin/env bash
# Attach the unversioned aliases the in-app update check resolves by name.
#
# `crates/ui-egui/src/update.rs` (`asset_name`) asks for some assets **without** a version:
#   intcad-linux-<arch>.AppImage / .deb / .rpm / .flatpak / .snap
#   intcad-windows-<arch>.msi, intcad-windows-<arch>-portable.zip
#   intcad-macos-universal.dmg
# The tarball and the standalone `.exe` are looked up *with* the version, so they get no alias, and
# `.zsync` files are never fetched by name.
#
# A release that ships only `intcad-<version>-…` files leaves MSI, DMG, AppImage, deb, rpm, Flatpak,
# Snap and portable-zip users unable to update: the update check finds no asset and stops. This
# script writes the aliases as **copies** — a hard link would change its bytes if the versioned file
# were rebuilt in place later — and refuses to replace an alias that already holds different bytes.
#
# Usage: packaging/aliases.sh <dist directory> <version>
set -euo pipefail
trap 'echo "aliases.sh: line $LINENO: \"$BASH_COMMAND\" failed (exit $?)" >&2' ERR

dir="${1:?usage: packaging/aliases.sh <dist directory> <version>}"
version="${2:?usage: packaging/aliases.sh <dist directory> <version>}"
version="${version#v}"
[ -d "$dir" ] || { echo "aliases.sh: no such folder: $dir" >&2; exit 1; }

# The suffixes whose asset name carries no version (see `asset_name`).
aliased() {
  case "$1" in
    *.AppImage | *.deb | *.rpm | *.flatpak | *.snap | *.msi | *-portable.zip | *.dmg) return 0 ;;
    *) return 1 ;;
  esac
}

made=0
already=0
for file in "$dir"/*; do
  # An alias is published as a file; a symlink source would make one that `checksums.sh` refuses and
  # that a later cleanup could empty.
  if [ -L "$file" ]; then
    echo "aliases.sh: $file is a symlink, so its alias would be one too; put the file itself in $dir" >&2
    exit 1
  fi
  [ -f "$file" ] || continue
  name="$(basename "$file")"
  case "$name" in
    intcad-"$version"-*) ;;
    *) continue ;;
  esac
  aliased "$name" || continue
  alias_name="intcad-${name#intcad-"$version"-}"
  target="$dir/$alias_name"
  if [ -e "$target" ] || [ -L "$target" ]; then
    if [ -L "$target" ] || [ ! -f "$target" ]; then
      echo "aliases.sh: $alias_name exists and is not a regular file; remove it and rerun" >&2
      exit 1
    fi
    if cmp -s "$file" "$target"; then
      already=$((already + 1))
      continue
    fi
    echo "aliases.sh: $alias_name already exists with different bytes; remove it or pick another version" >&2
    exit 1
  fi
  # `-n` so a file that appears between the check above and the copy is never overwritten.
  cp -n "$file" "$target"
  cmp -s "$file" "$target" || {
    echo "aliases.sh: $alias_name was not written to match $name; remove it and rerun" >&2
    exit 1
  }
  echo "aliased $alias_name"
  made=$((made + 1))
done

if [ "$made" -eq 0 ] && [ "$already" -eq 0 ]; then
  echo "aliases.sh: no versioned asset needing an alias in $dir (for $version)" >&2
fi
