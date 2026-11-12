#!/usr/bin/env bash
# The hosted ubuntu-24.04 image carries multi-GB SDKs this repository never
# uses. A full validation compiles the whole workspace for the native lane and
# again for the nextest archive; together with the restored Cargo cache that
# filled the root filesystem and failed the build with
# "No space left on device (os error 28)".
#
# Remove only large, unused image payloads before Cargo restores its cache.
# Cleanup is best effort: a missing directory or a failed removal must never
# turn a test result into a build failure.
set -uo pipefail

available_kib() {
  df -Pk / | awk 'NR == 2 { print $4 }'
}

before=$(available_kib)
for path in \
  /usr/local/lib/android \
  /usr/share/dotnet \
  /opt/ghc \
  /opt/hostedtoolcache/CodeQL \
  /usr/local/share/boost \
  /usr/local/.ghcup \
  /usr/share/swift \
  /opt/az
do
  if [ -e "$path" ]; then
    sudo rm -rf -- "$path" || echo "free-disk: could not remove $path"
  fi
done
sudo apt-get clean || true
after=$(available_kib)
echo "free-disk: $((before / 1024)) MiB -> $((after / 1024)) MiB available on /"
df -h /
