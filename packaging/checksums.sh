#!/usr/bin/env bash
# Write the checksum list a release publishes, under both names clients fetch.
#
# `crates/ui-egui/src/update/install.rs` (`SUMS`), `packaging/web-channels.sh` and
# `packaging/flatpak-site.sh` fetch `SHA256SUMS.txt`. Releases assembled by hand before this
# script carried `SHA256SUMS`, so the same bytes are also written under that name. Every hash is
# checked against the files before anything replaces an earlier list, and both names are checked
# afterwards.
#
# Usage: packaging/checksums.sh <dist directory> [--require <name>]...
#
#   --require <name>   fail unless that exact file name is listed. Give one per release asset the
#                      channel promises (`--require intcad-0.1.3-linux-x86_64.tar.gz`), including
#                      the unversioned aliases clients resolve by name: a list that omits one
#                      leaves those users unable to update even with the file attached.
#
# A directory holding no regular files, a name with a newline, and a top-level symlink (which is
# silently not a release file) all stop it.
set -euo pipefail
trap 'echo "checksums.sh: line $LINENO: \"$BASH_COMMAND\" failed (exit $?)" >&2' ERR

dir=
required=()
while [ $# -gt 0 ]; do
  case "$1" in
    --require) required+=("${2:-}"); shift 2 || { echo "checksums.sh: --require needs a name" >&2; exit 2; } ;;
    -h | --help) sed -n '2,20p' "$0"; exit 0 ;;
    --*) echo "checksums.sh: unknown option $1" >&2; exit 2 ;;
    *) [ -z "$dir" ] || { echo "checksums.sh: one directory at a time" >&2; exit 2; }; dir="$1"; shift ;;
  esac
done

dir="${dir:?usage: packaging/checksums.sh <dist directory> [--require <name>]...}"
if [ ! -d "$dir" ]; then
  echo "checksums.sh: no such folder: $dir" >&2
  exit 1
fi

# The lists themselves, a signature over one of them, and the per-package evidence the release
# workflow writes beside the artifacts are not download assets.
is_list() {
  case "$1" in
    SHA256SUMS.txt | SHA256SUMS | SHA256SUMS.txt.new | SHA256SUMS.new | SHA256SUMS.txt.minisig | SHA256SUMS.minisig | build.json | contents.json) return 0 ;;
    *) return 1 ;;
  esac
}

# The names are read into a file first: a failing `find` inside a process substitution would
# otherwise be invisible and produce a short list that still looks successful.
found="$(mktemp "${TMPDIR:-/tmp}/checksums-find.XXXXXX")"
# The temporary list is made in `$dir` only after the scan: a file created before it would be
# found as a release file and listed with a hash of itself.
tmp=
odd=
trap 'rm -f "$found" ${tmp:+"$tmp"} ${odd:+"$odd"} "$dir/SHA256SUMS.txt.new" "$dir/SHA256SUMS.new"' EXIT
find "$dir" -mindepth 1 -maxdepth 1 -type f -print0 | LC_ALL=C sort -z >"$found"

files=()
while IFS= read -r -d '' file; do
  # The path, not `basename`: command substitution removes trailing newlines, which would hide a
  # name that cannot appear in a checksum line.
  case "$file" in
    *$'\n'* | *$'\r'*) echo "checksums.sh: a file name holds a newline, which a checksum line cannot: $file" >&2; exit 1 ;;
  esac
  name="${file##*/}"
  is_list "$name" && continue
  files+=("$name")
done <"$found"

if [ "${#files[@]}" -eq 0 ]; then
  echo "checksums.sh: no release files in $dir" >&2
  exit 1
fi

# Anything at the top level that is not a regular file would be left out of the list without a sign:
# a symlink, a subdirectory (what `gh run download` makes), a socket. Refuse instead, and say what
# to do — a short list that still looks successful is the failure this check exists to prevent.
# Its own file too, for the same reason as the asset scan: a failing `find` must stop the run.
odd="$(mktemp "${TMPDIR:-/tmp}/checksums-odd.XXXXXX")"
find "$dir" -mindepth 1 -maxdepth 1 ! -type f -print0 >"$odd"

while IFS= read -r -d '' entry; do
  name="${entry##*/}"
  is_list "$name" && continue
  if [ -d "$entry" ]; then
    echo "checksums.sh: $entry is a folder, so its files would not be listed; flatten the release assets into $dir first" >&2
  elif [ -L "$entry" ]; then
    echo "checksums.sh: $entry is a symlink, so it would not be listed; put the file itself in $dir" >&2
  else
    echo "checksums.sh: $entry is not a regular file, so it would not be listed" >&2
  fi
  exit 1
done <"$odd"

# The two published names must be regular files: overwriting a symlink or a directory here would
# write the list somewhere else while still looking like success.
for name in SHA256SUMS.txt SHA256SUMS; do
  if [ -e "$dir/$name" ] && [ ! -f "$dir/$name" ]; then
    echo "checksums.sh: $dir/$name exists and is not a regular file" >&2
    exit 1
  fi
  if [ -L "$dir/$name" ]; then
    echo "checksums.sh: $dir/$name is a symlink; remove it and write a real list" >&2
    exit 1
  fi
done

tmp="$(mktemp "${TMPDIR:-/tmp}/checksums-list.XXXXXX")"
(
  cd "$dir"
  for file in "${files[@]}"; do
    printf '%s  %s\n' "$(sha256sum -- "$file" | cut -d' ' -f1)" "$file"
  done
) >"$tmp"

# The list is checked before it replaces anything: a wrong hash never reaches a published name.
# The names are relative to `$dir`, so the check runs there; the list itself comes on stdin
# because it does not live in `$dir` yet (a file made there first would list itself).
verify() {
  ( cd "$dir" && sha256sum --check --strict <"$1" >/dev/null )
}
verify "$tmp"

missing=()
for name in "${required[@]}"; do
  # A line is `<64 hex>  <name>`, so compare the trailing name field whole: a prefix must not
  # pass (`intcad-windows-x64.msi` matching only `intcad-windows-x64.msi.part`).
  # `-v` would expand backslashes in the name; the environment passes it as it is.
  WANT="$name" awk 'index($0, "  " ENVIRON["WANT"]) == length($0) - length(ENVIRON["WANT"]) - 1 { found = 1 } END { exit !found }' "$tmp" \
    || missing+=("$name")
done
if [ "${#missing[@]}" -gt 0 ]; then
  echo "checksums.sh: the list would omit these release assets: ${missing[*]}" >&2
  exit 1
fi

# Both names are put in place from copies, so a failure on the second one does not leave the first
# updated and the second stale. The `.new` names are removed first: copying into an existing symlink
# or hard link would write somewhere else and still look like it worked. If either rename fails,
# both published names are removed rather than left describing different sets of files.
for name in SHA256SUMS.txt SHA256SUMS; do
  rm -f "$dir/$name.new"
  cp "$tmp" "$dir/$name.new"
  chmod 644 "$dir/$name.new"
done
for name in SHA256SUMS.txt SHA256SUMS; do
  if ! mv "$dir/$name.new" "$dir/$name"; then
    rm -f "$dir/SHA256SUMS.txt" "$dir/SHA256SUMS"
    echo "checksums.sh: could not put $dir/$name in place; no checksum list was left" >&2
    exit 1
  fi
done
cmp -s "$dir/SHA256SUMS.txt" "$dir/SHA256SUMS" || {
  echo "checksums.sh: the two published names do not hold the same bytes" >&2
  exit 1
}

# Both names, as published: the same lines and the same bytes, each verified in its published
# place (the output names every file, so the run shows what was checked).
cmp "$dir/SHA256SUMS.txt" "$dir/SHA256SUMS"
(cd "$dir" && sha256sum --check --strict SHA256SUMS.txt && sha256sum --check --strict SHA256SUMS >/dev/null)
echo "checksums.sh: ${#files[@]} files in $dir/SHA256SUMS.txt and $dir/SHA256SUMS"
