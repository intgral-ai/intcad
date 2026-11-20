#!/usr/bin/env bash
# All Pages deployments include the book, web app, and published Flatpak repository.
# Shared by docs updates and release publication; no other workflow replaces the site.
set -euo pipefail
trap 'echo "flatpak-site.sh: line $LINENO: \"$BASH_COMMAND\" failed (exit $?)" >&2' ERR

site="${1:?site directory required}"
tag="${RELEASE_TAG:-}"
app_id=ai.intgral.intcad
if [ -z "$tag" ]; then
  tag="$(gh api "repos/$GITHUB_REPOSITORY/releases?per_page=100" \
    --jq '[.[] | select(.draft == false and .prerelease == false)][0].tag_name // empty')"
fi
# Before the first publication the docs and browser app can still deploy.
if [ -z "$tag" ]; then exit 0; fi
if ! printf '%s' "$tag" | grep -Eq '^v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(-((0|[1-9][0-9]*|[0-9]*[A-Za-z-][0-9A-Za-z-]*)(\.(0|[1-9][0-9]*|[0-9]*[A-Za-z-][0-9A-Za-z-]*))*))?(\+[0-9A-Za-z-]+(\.[0-9A-Za-z-]+)*)?$'; then
  echo "invalid release tag: $tag" >&2
  exit 1
fi
published="$(gh api "repos/$GITHUB_REPOSITORY/releases/tags/$tag" \
  --jq 'select(.draft == false and .published_at != null) | .tag_name')"
test "$published" = "$tag"
test -s "$site/index.html"
test -s "$site/app/index.html"

version="${tag#v}"
bundles="$(mktemp -d)"
gh release download "$tag" --repo "$GITHUB_REPOSITORY" --pattern SHA256SUMS.txt --dir "$bundles"
repo_dir="$site/flatpak/repo"
mkdir -p "$repo_dir"
ostree --repo="$repo_dir" init --mode=archive-z2
base_url="${PAGES_BASE_URL%/}"
# Pages reports its base URL as http:// for a custom domain; the repository is always fetched over
# HTTPS (the remote URL is stored by every client that adds it).
base_url="https://${base_url#*://}"
previous_url="$base_url/flatpak/repo"
if curl --fail --silent --location --output /dev/null "$previous_url/summary"; then
  ostree --repo="$repo_dir" remote add --no-gpg-verify previous "$previous_url"
  ostree --repo="$repo_dir" pull --mirror previous
  ostree --repo="$repo_dir" remote delete previous
  # Keep only Intcad's own refs. `ostree refs --delete` matches a PREFIX, so first
  # refuse any foreign prefix that would also match an Intcad branch (including the
  # bare app id): fail closed instead of deleting Intcad refs or publishing foreign ones.
  refs="$(ostree --repo="$repo_dir" refs)"
  intcad_refs=()
  foreign_refs=()
  while IFS= read -r ref; do
    [ -n "$ref" ] || continue
    case "$ref" in
      app/*) ;;
      *) echo "error: unexpected ref '$ref' in the Flatpak repository" >&2; exit 1 ;;
    esac
    case "$ref" in
      *[!A-Za-z0-9._/+-]*) echo "error: unsafe ref '$ref' in the Flatpak repository" >&2; exit 1 ;;
    esac
    case "$ref" in
      "app/$app_id/"*) intcad_refs+=("$ref") ;;
      *) foreign_refs+=("$ref") ;;
    esac
  done <<< "$refs"
  for foreign in "${foreign_refs[@]}"; do
    for intcad in "${intcad_refs[@]}"; do
      case "$intcad" in
        "$foreign"*)
          echo "error: ref '$foreign' is a prefix of Intcad ref '$intcad'; refusing to delete or publish" >&2
          exit 1
          ;;
      esac
    done
  done
  deleted=()
  for foreign in "${foreign_refs[@]}"; do
    covered=0
    for prefix in "${deleted[@]}"; do
      case "$foreign" in "$prefix"*) covered=1; break ;; esac
    done
    [ "$covered" = 1 ] && continue
    ostree --repo="$repo_dir" refs --delete "$foreign"
    deleted+=("$foreign")
  done
fi
published_id=$app_id
for arch in x86_64 aarch64; do
  asset="intcad-$version-linux-$arch.flatpak"
  gh release download "$tag" --repo "$GITHUB_REPOSITORY" --pattern "$asset" --dir "$bundles"
  (
    cd "$bundles"
    awk -v file="$asset" '$2 == file || $2 == "*" file { print; found=1 } END { exit !found }' \
      SHA256SUMS.txt | sha256sum --check -
  )
  validation_repo="$(mktemp -d)"
  ostree --repo="$validation_repo" init --mode=archive-z2
  flatpak build-import-bundle "$validation_repo" "$bundles/$asset"
  refs="$(ostree --repo="$validation_repo" refs)"
  if ! printf '%s\n' "$refs" | grep -Fxq "app/$app_id/$arch/stable"; then
    echo "error: $asset does not hold app/$app_id/$arch/stable; refs:" >&2
    printf '%s\n' "$refs" >&2
    exit 1
  fi
  echo "$asset: app/$app_id/$arch/stable"
  flatpak build-import-bundle "$repo_dir" "$bundles/$asset"
done
flatpak build-update-repo --title="Intcad" --comment='Official Intcad releases' \
  --description='Official Flatpak repository for Intcad' \
  --homepage="https://github.com/$GITHUB_REPOSITORY" --default-branch=stable "$repo_dir"
test -s "$repo_dir/summary"
cat >"$site/flatpak/solidcraft.flatpakrepo" <<META
[Flatpak Repo]
Title=Intcad
Comment=Official Intcad releases
Description=Official Flatpak repository for Intcad
Url=$base_url/flatpak/repo/
Homepage=https://github.com/$GITHUB_REPOSITORY
META
cat >"$site/flatpak/index.html" <<HTML
<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1"><title>Intcad Flatpak repository</title></head><body><main><h1>Intcad Flatpak repository</h1><p>Latest published release: <strong>$tag</strong></p><p><a href="solidcraft.flatpakrepo">Add the Intcad repository</a></p><p>Flathub provides the required Freedesktop runtime. Add both remotes to the same user installation:</p><pre>flatpak remote-add --user --if-not-exists flathub https://dl.flathub.org/repo/flathub.flatpakrepo
flatpak remote-add --user --if-not-exists solidcraft $base_url/flatpak/solidcraft.flatpakrepo
flatpak install --user solidcraft $published_id
flatpak run $published_id</pre></main></body></html>
HTML
# Compatibility shim (docs/compatibility.md): links and instructions from before the rename point
# at solvecraft.flatpakrepo. Existing remotes keep working either way: they store the repo URL.
cp "$site/flatpak/solidcraft.flatpakrepo" "$site/flatpak/solvecraft.flatpakrepo"
touch "$site/.nojekyll"
