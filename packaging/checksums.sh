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

# The lists themselves, and a signature over one of them, are not release files.
is_list() {
  case "$1" in
    SHA256SUMS.txt | SHA256SUMS | SHA256SUMS.txt.minisig | SHA256SUMS.minisig) return 0 ;;
    *) return 1 ;;
  esac
}

# The names are read into a file first: a failing `find` inside a process substitution would
# otherwise be invisible and produce a short list that still looks successful.
found="$(mktemp "${TMPDIR:-/tmp}/checksums-find.XXXXXX")"
# The temporary list is made in `$dir` only after the scan: a file created before it would be
# found as a release file and listed with a hash of itself.
tmp=
trap 'rm -f "$found" ${tmp:+"$tmp"}' EXIT
find "$dir" -mindepth 1 -maxdepth 1 -type f -print0 | LC_ALL=C sort -z >"$found"

files=()
while IFS= read -r -d '' file; do
  name="$(basename "$file")"
  case "$name" in
    *$'\n'* | *$'\r'*) echo "checksums.sh: a file name holds a newline, which a checksum line cannot: $name" >&2; exit 1 ;;
  esac
  is_list "$name" && continue
  files+=("$name")
done <"$found"

if [ "${#files[@]}" -eq 0 ]; then
  echo "checksums.sh: no release files in $dir" >&2
  exit 1
fi

# A symlink at the top level is not a release file and would silently be left out of the list.
while IFS= read -r -d '' link; do
  is_list "$(basename "$link")" && continue
  echo "checksums.sh: $link is a symlink, so it would not be listed; put the file itself in $dir" >&2
  exit 1
done < <(find "$dir" -mindepth 1 -maxdepth 1 -type l -print0)

tmp="$(mktemp "${TMPDIR:-/tmp}/checksums-list.XXXXXX")"
trap 'rm -f "$found" "$tmp"' EXIT
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
  # A line is `<hash>  <name>`, so compare the name field whole: a prefix would pass for
  # `intcad-windows-x64.msi` when only `intcad-windows-x64.msi.part` were listed.
  awk -v want="$name" 'index($0, "  " want) == length($0) - length(want) + 1 { found = 1 } END { exit !found }' "$tmp" \
    || missing+=("$name")
done
if [ "${#missing[@]}" -gt 0 ]; then
  echo "checksums.sh: the list would omit these release assets: ${missing[*]}" >&2
  exit 1
fi

mv "$tmp" "$dir/SHA256SUMS.txt"
cp "$dir/SHA256SUMS.txt" "$dir/SHA256SUMS"
# `mktemp` makes 0600; a published list is read by anyone.
chmod 644 "$dir/SHA256SUMS.txt" "$dir/SHA256SUMS"

# Both names, as published: the same lines and the same bytes, each verified in its published
# place (the output names every file, so the run shows what was checked).
cmp "$dir/SHA256SUMS.txt" "$dir/SHA256SUMS"
(cd "$dir" && sha256sum --check --strict SHA256SUMS.txt && sha256sum --check --strict SHA256SUMS >/dev/null)
echo "checksums.sh: ${#files[@]} files in $dir/SHA256SUMS.txt and $dir/SHA256SUMS"
