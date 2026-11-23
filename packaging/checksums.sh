#!/usr/bin/env bash
# Write the checksum list a release publishes, under both names clients fetch.
#
# `crates/ui-egui/src/update/install.rs` (`SUMS`), `packaging/web-channels.sh` and
# `packaging/flatpak-site.sh` fetch `SHA256SUMS.txt`. Releases assembled by hand before this
# script carried `SHA256SUMS`, so the same bytes are also written under that name. Every hash is
# checked against the files before the script finishes.
#
# Usage: packaging/checksums.sh <dist directory>
set -euo pipefail
trap 'echo "checksums.sh: line $LINENO: \"$BASH_COMMAND\" failed (exit $?)" >&2' ERR

dir="${1:?usage: packaging/checksums.sh <dist directory>}"
if [ ! -d "$dir" ]; then
  echo "checksums.sh: no such folder: $dir" >&2
  exit 1
fi

# The lists themselves, and a signature over one of them, are not release files.
lists=(SHA256SUMS.txt SHA256SUMS SHA256SUMS.txt.minisig SHA256SUMS.minisig)
files=()
while IFS= read -r file; do
  name="$(basename "$file")"
  for list in "${lists[@]}"; do
    [ "$name" = "$list" ] && continue 2
  done
  files+=("$file")
done < <(find "$dir" -mindepth 1 -maxdepth 1 -type f -print | LC_ALL=C sort)

if [ "${#files[@]}" -eq 0 ]; then
  echo "checksums.sh: no release files in $dir" >&2
  exit 1
fi

tmp="$(mktemp "$dir/.SHA256SUMS.XXXXXX")"
trap 'rm -f "$tmp"' EXIT
(
  cd "$dir"
  for file in "${files[@]}"; do
    printf '%s  %s\n' "$(sha256sum "$(basename "$file")" | cut -d' ' -f1)" "$(basename "$file")"
  done
) >"$tmp"
mv "$tmp" "$dir/SHA256SUMS.txt"
cp "$dir/SHA256SUMS.txt" "$dir/SHA256SUMS"
# `mktemp` makes 0600; a published list is read by anyone.
chmod 644 "$dir/SHA256SUMS.txt" "$dir/SHA256SUMS"
trap - EXIT

(cd "$dir" && sha256sum --check --strict SHA256SUMS.txt)
echo "checksums.sh: $(wc -l <"$dir/SHA256SUMS.txt") files in $dir/SHA256SUMS.txt and $dir/SHA256SUMS"
