#!/usr/bin/env bash
# Build and package SolidCraft for macOS:
#
#   $DIST/intcad-<version>-macos-<arch>.dmg        Intcad.app (with intcad-cli inside) on a
#                                                      drag-to-Applications DMG with a drawn window
#                                                      (assets/platforms/macos/dmg)
#   $DIST/intcad-cli-<version>-macos-<arch>.zip    the headless CLI with the licence files
#
# Usage: packaging/macos/package.sh [--arch universal|aarch64|x86_64] [--skip-build] [--sign] [--with-mastra]
#                                   [--with-mastra-agent]
#        packaging/macos/package.sh --build-only <rust-target>
#
# --with-mastra-agent (or SOLIDCRAFT_WITH_MASTRA_AGENT=1) puts the built-in agent's folder into
# the app, for a test package: --arch aarch64 only. docs/mastra-packaging.md says how it is built.
#
# --build-only builds just that target (aarch64-apple-darwin or x86_64-apple-darwin) and stops,
# so the release workflow can build the two halves of the universal binary on separate runners;
# a later `--arch universal --skip-build` run joins the binaries it finds under
# $CARGO_TARGET_DIR/<target>/release/ and packages them.
#
# Signing: with --sign, MACOS_SIGN_IDENTITY (a "Developer ID Application: ..." name or its SHA-1
# hash, found in the keychain) signs the app, the CLI and the DMG with the hardened runtime and a
# secure timestamp. With the App Store Connect API key as well (APPLE_API_KEY_PATH = the .p8 file,
# APPLE_API_KEY_ID, APPLE_API_ISSUER) the app, the DMG and the CLI zip are notarized; the app
# and the DMG are stapled. Without --sign the app is ad-hoc signed, not notarized: first launch needs
# right-click > Open.
# Adapted from PhotoCraft's packaging (storytold/photocraft, Apache-2.0; see ATTRIBUTION.md).
set -euo pipefail
# shellcheck source=../env.sh
. "$(dirname "${BASH_SOURCE[0]}")/../env.sh"
# The same reverse-DNS id as the Linux APP_ID.
MACOS_BUNDLE_ID=ai.intgral.intcad

# sign_code <path> [identifier] [entitlements]: Developer ID with the hardened runtime and a secure
# timestamp, or the ad-hoc signature (Apple silicon refuses unsigned code) without an identity.
sign_code() {
  local path="$1" identifier="${2:-}" entitlements="${3:-}"
  local args=(--force)
  if [ -n "$SIGN_IDENTITY" ]; then
    args+=(--sign "$SIGN_IDENTITY" --options runtime --timestamp)
  else
    args+=(--sign - --timestamp=none)
  fi
  if [ -n "$identifier" ]; then args+=(--identifier "$identifier"); fi
  if [ -n "$entitlements" ] && [ -n "$SIGN_IDENTITY" ]; then args+=(--entitlements "$entitlements"); fi
  codesign "${args[@]}" "$path"
}

# notarize <path>: submit to Apple and wait up to 30 minutes; a rejection or timeout fails the
# build, and a rejection prints Apple's log.
notarize() {
  local out
  # Not fatal here: the status check below is the only failure path, so the log is always fetched.
  out="$(xcrun notarytool submit "$1" --key "$APPLE_API_KEY_PATH" --key-id "$APPLE_API_KEY_ID" \
    --issuer "$APPLE_API_ISSUER" --wait --timeout 30m --output-format json || true)"
  echo "$out"
  local id
  id="$(printf '%s' "$out" | /usr/bin/plutil -extract id raw -o - - 2>/dev/null || true)"
  if [ "$(printf '%s' "$out" | /usr/bin/plutil -extract status raw -o - - 2>/dev/null || true)" != Accepted ]; then
    if [ -n "$id" ]; then
      xcrun notarytool log "$id" --key "$APPLE_API_KEY_PATH" --key-id "$APPLE_API_KEY_ID" --issuer "$APPLE_API_ISSUER" >&2 || true
    fi
    echo "notarization of $1 was not accepted" >&2
    return 1
  fi
}

# staple <path>: stapling can fail with error 65 for a while after Apple accepts a submission, so
# try a few times.
staple() {
  local i
  for i in 1 2 3 4 5 6; do
    if xcrun stapler staple "$1"; then return 0; fi
    echo "stapler staple failed (attempt $i), retrying" >&2
    sleep "${STAPLE_RETRY_SLEEP:-20}"
  done
  echo "could not staple $1" >&2
  return 1
}

ARCH=universal
SKIP_BUILD=0
BUILD_ONLY=
SIGN=0
WITH_MASTRA="${SOLIDCRAFT_WITH_MASTRA:-0}"
WITH_MASTRA_AGENT="${SOLIDCRAFT_WITH_MASTRA_AGENT:-0}"
while [ $# -gt 0 ]; do
  case "$1" in
    --arch) ARCH="$2"; shift 2 ;;
    --skip-build) SKIP_BUILD=1; shift ;;
    --build-only) BUILD_ONLY="$2"; shift 2 ;;
    --sign) SIGN=1; shift ;;
    --with-mastra) WITH_MASTRA=1; shift ;;
    --without-mastra) WITH_MASTRA=0; shift ;;
    --with-mastra-agent) WITH_MASTRA_AGENT=1; shift ;;
    -h | --help) sed -n '2,27p' "$0"; exit 0 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done
case "$ARCH" in
  universal) TARGETS=(aarch64-apple-darwin x86_64-apple-darwin) ;;
  aarch64) TARGETS=(aarch64-apple-darwin) ;;
  x86_64) TARGETS=(x86_64-apple-darwin) ;;
  *) echo "unknown --arch $ARCH" >&2; exit 2 ;;
esac

if [ "$WITH_MASTRA_AGENT" = 1 ] && [ "$ARCH" != aarch64 ] && [ -z "$BUILD_ONLY" ]; then
  echo "--with-mastra-agent is for --arch aarch64: the agent's folder is built for one CPU, and only Apple silicon is offered" >&2; exit 2
fi
if [ -n "$BUILD_ONLY" ] && [ "$SKIP_BUILD" = 1 ]; then
  echo "--skip-build and --build-only exclude each other" >&2; exit 2
fi
if [ -n "$BUILD_ONLY" ] && [ "$SIGN" = 1 ]; then
  echo "--build-only builds nothing to sign: drop --sign" >&2; exit 2
fi
SIGN_IDENTITY=
NOTARIZE=0
if [ "$SIGN" = 1 ]; then
  SIGN_IDENTITY="${MACOS_SIGN_IDENTITY:-}"
  if [ -z "$SIGN_IDENTITY" ]; then echo "--sign needs MACOS_SIGN_IDENTITY" >&2; exit 2; fi
  if [ -n "${APPLE_API_KEY_PATH:-}" ] && [ -n "${APPLE_API_KEY_ID:-}" ] && [ -n "${APPLE_API_ISSUER:-}" ]; then
    NOTARIZE=1
  else
    echo "==> Developer ID signing without notarization: the APPLE_API_* settings are incomplete"
  fi
else
  echo "==> no --sign: ad-hoc signing, no notarization"
fi

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# Keep in sync with LSMinimumSystemVersion in Info.plist.in.
export MACOSX_DEPLOYMENT_TARGET=11.0
SHORT_VERSION="${VERSION%%-*}"
WORK="$CARGO_TARGET_DIR/macos-package"
APP="$WORK/Intcad.app"
DMG="$DIST/intcad-$VERSION-macos-$ARCH.dmg"
CLI_ZIP="$DIST/intcad-cli-$VERSION-macos-$ARCH.zip"

if [ -n "$BUILD_ONLY" ]; then
  case "$BUILD_ONLY" in
    aarch64-apple-darwin | x86_64-apple-darwin) ;;
    *) echo "unknown --build-only target $BUILD_ONLY" >&2; exit 2 ;;
  esac
  echo "==> SolidCraft $VERSION for macOS ($BUILD_ONLY, build only)"
  (cd "$ROOT" && cargo build --release --locked -p solidcraft -p solidcraft-cli --target "$BUILD_ONLY")
  exit 0
fi

echo "==> SolidCraft $VERSION for macOS ($ARCH)"
if [ "$SKIP_BUILD" = 0 ]; then
  args=()
  for t in "${TARGETS[@]}"; do args+=(--target "$t"); done
  (cd "$ROOT" && cargo build --release --locked -p solidcraft -p solidcraft-cli "${args[@]}")
fi

# An earlier run may have left the built-in agent's folder, which nothing may write.
if [ -d "$WORK" ]; then chmod -R u+w "$WORK"; fi
rm -rf "$WORK"
mkdir -p "$WORK/bin"
for bin in intcad intcad-cli; do
  inputs=()
  for t in "${TARGETS[@]}"; do inputs+=("$CARGO_TARGET_DIR/$t/release/$bin"); done
  lipo -create -output "$WORK/bin/$bin" "${inputs[@]}"
  lipo -info "$WORK/bin/$bin"
done

# ---- Intcad.app ----------------------------------------------------------------------------
echo "==> assembling $APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "$WORK/bin/intcad" "$APP/Contents/MacOS/Intcad"
# intcad-cli next to the app, as every other package has it: the agent chat starts it as the
# agent's MCP server (`intcad-cli mcp --connect`) and looks for it beside the running program.
cp "$WORK/bin/intcad-cli" "$APP/Contents/MacOS/intcad-cli"
# The icon file with the product's mark (regeneration: assets/README.md). No layered icon is
# installed: the Assets.car under assets/ still holds the upstream artwork, and compiling one
# for the mark needs Xcode 26. macOS 26 would show a layered icon in place of the icon file.
cp "$ROOT/assets/icons/app/macos/exports/icon.icns" "$APP/Contents/Resources/Intcad.icns"
mkdir -p "$APP/Contents/Resources/Licenses"
copy_docs "$APP/Contents/Resources/Licenses"
# The third-party notices of the Assistant's page (Mastra's components, npm packages, fonts),
# which the app carries built in as well (Help > About > Licences).
cp "$ROOT/packages/assistant-web/THIRD-PARTY-NOTICES.txt" "$APP/Contents/Resources/Licenses/ASSISTANT-THIRD-PARTY-NOTICES.txt"
cp "$ROOT/packaging/notices/WEB-VIEW-PLATFORM-NOTICES.txt" "$APP/Contents/Resources/Licenses/WEB-VIEW-PLATFORM-NOTICES.txt"
# The inventory of what that page was built from. The page itself is embedded in the executable
# (include_bytes!); packaging/macos/verify-assistant-bundle.py checks both in the finished app.
mkdir -p "$APP/Contents/Resources/Assistant"
cp "$ROOT/packages/assistant-web/inventory.json" "$APP/Contents/Resources/Assistant/inventory.json"

# ---- Mastra runtime (opt-in; application-relative, pinned Node) ---------------------------------
# Bundling is opt-in because the artifact is still unapproved (DQ-7, #15). A universal app needs
# both per-slice artifacts: SOLIDCRAFT_MASTRA_ARTIFACT_AARCH64 and _X86_64. A single-arch app
# uses SOLIDCRAFT_MASTRA_ARTIFACT.
stage_mastra() {
  local target="$1" variable="$2" dest="$3"
  local artifact="${!variable:-}"
  if [ -z "$artifact" ]; then
    echo "error: $variable is not set; the $target Mastra runtime is required for macOS $ARCH with --with-mastra (unapproved until #15)" >&2
    exit 2
  fi
  "$SCRIPT_DIR/../mastra/stage.sh" --target "$target" --artifact "$artifact" --dest "$dest"
}
if [ "$WITH_MASTRA" = 1 ]; then
  if [ "$ARCH" = universal ]; then
    stage_mastra aarch64-apple-darwin SOLIDCRAFT_MASTRA_ARTIFACT_AARCH64 "$APP/Contents/Resources/mastra/aarch64-apple-darwin"
    stage_mastra x86_64-apple-darwin SOLIDCRAFT_MASTRA_ARTIFACT_X86_64 "$APP/Contents/Resources/mastra/x86_64-apple-darwin"
  else
    stage_mastra "${TARGETS[0]}" SOLIDCRAFT_MASTRA_ARTIFACT "$APP/Contents/Resources/mastra"
  fi
  mkdir -p "$APP/Contents/Resources/Licenses"
  mastra_licenses="$(find "$APP/Contents/Resources/mastra" -path '*/licenses/LICENSES.txt' -print -quit)"
  mastra_notice="$(find "$APP/Contents/Resources/mastra" -path '*/notices/NOTICE.txt' -print -quit)"
  if [ -n "$mastra_licenses" ]; then cp "$mastra_licenses" "$APP/Contents/Resources/Licenses/MASTRA-LICENSES.txt"; fi
  if [ -n "$mastra_notice" ]; then cp "$mastra_notice" "$APP/Contents/Resources/Licenses/MASTRA-NOTICE.txt"; fi
else
  warn "the built-in Mastra workflow is not bundled (opt-in; the runtime artifact is unapproved, DQ-7/#15)"
fi

# ---- Built-in agent (opt-in, for test packages) -------------------------------------------------
# Its own folder beside `mastra/`: the app starts the Node in it with agent/main.mjs when no
# environment variable names an agent. stage-agent.sh checks the folder before copying it.
AGENT_DIR="$APP/Contents/Resources/mastra-agent"
if [ "$WITH_MASTRA_AGENT" = 1 ]; then
  "$SCRIPT_DIR/../mastra/stage-agent.sh" --target "${TARGETS[0]}" --dest "$AGENT_DIR"
  cp "$AGENT_DIR/licenses/THIRD-PARTY-LICENSES.txt" "$APP/Contents/Resources/Licenses/MASTRA-AGENT-THIRD-PARTY-LICENSES.txt"
  cp "$AGENT_DIR/licenses/NODE-LICENSE.txt" "$APP/Contents/Resources/Licenses/MASTRA-AGENT-NODE-LICENSE.txt"
fi

sed -e "s/@VERSION@/$VERSION/g" -e "s/@SHORT_VERSION@/$SHORT_VERSION/g" "$ROOT/assets/platforms/macos/Info.plist.in" >"$APP/Contents/Info.plist"
plutil -lint "$APP/Contents/Info.plist"
printf 'APPL????' >"$APP/Contents/PkgInfo"
# Signature over the whole bundle. Nested code is signed first: the bundle signature refuses an
# unsigned helper in Contents/MacOS. The entitlements go on the bundle (they apply to the main
# executable): the 3D mouse driver is loaded by the app, not by the CLI.
sign_code "$APP/Contents/MacOS/intcad-cli" "$MACOS_BUNDLE_ID.cli"
# The bundled pinned Node is nested executable code: sign it before the app bundle.
if [ -d "$APP/Contents/Resources/mastra" ]; then
  find "$APP/Contents/Resources/mastra" -type f -name node -print0 | while IFS= read -r -d '' node; do
    sign_code "$node" "$MACOS_BUNDLE_ID.mastra-node" "$SCRIPT_DIR/node-entitlements.plist"
  done
fi
# The built-in agent's Node, and the native libraries its packages load, are nested code too.
if [ -d "$AGENT_DIR" ]; then
  find "$AGENT_DIR/node_modules" -type f \( -name '*.node' -o -name '*.dylib' \) -print0 | while IFS= read -r -d '' library; do
    sign_code "$library"
  done
  sign_code "$AGENT_DIR/node" "$MACOS_BUNDLE_ID.mastra-agent-node" "$SCRIPT_DIR/node-entitlements.plist"
  # Nothing in the folder may be written once it is in the app: the agent keeps what it writes
  # in the user's data folder, and a signed app must not change its own contents.
  chmod -R a-w "$AGENT_DIR"
  # The whole app as it will ship holds nothing that looks like a secret or a stored sign-in.
  node "$ROOT/packages/mastra-runtime/scripts/agent-package.mjs" scan "$APP"
fi
sign_code "$APP" "" "$SCRIPT_DIR/entitlements.plist"
codesign --verify --strict --deep --verbose=2 "$APP"
if [ -n "$SIGN_IDENTITY" ]; then
  # The hardened runtime and the library-validation exception must be on the signed app.
  # Match a captured string, not a pipe: grep -q closing the pipe can SIGPIPE codesign under pipefail.
  info="$(codesign --display --verbose=2 "$APP" 2>&1)"
  grep -q 'flags=.*runtime' <<<"$info"
  info="$(codesign --display --entitlements - --xml "$APP" 2>/dev/null)"
  grep -q 'com.apple.security.cs.disable-library-validation' <<<"$info"
fi
test -x "$APP/Contents/MacOS/intcad-cli" || { echo "intcad-cli is missing from $APP" >&2; exit 1; }
if [ "$NOTARIZE" = 1 ]; then
  # Staple the app itself, so a copy dragged out of the DMG opens offline too.
  echo "==> notarizing $APP"
  ditto -c -k --keepParent "$APP" "$WORK/Intcad-app.zip"
  notarize "$WORK/Intcad-app.zip"
  staple "$APP"
  xcrun stapler validate "$APP"
fi

# ---- DMG ---------------------------------------------------------------------------------------
echo "==> building $DMG"
DMG_ART="$ROOT/assets/platforms/macos/dmg"
# dmgbuild (hash-pinned, packaging/macos/dmg-requirements.txt) writes the window layout, the
# background and the icon positions into the image without a Finder session.
VENV="$WORK/dmg-venv"
python3 -m venv "$VENV"
"$VENV/bin/pip" install --quiet --disable-pip-version-check --require-hashes -r "$ROOT/packaging/macos/dmg-requirements.txt"
rm -f "$DMG"
"$VENV/bin/dmgbuild" -s "$DMG_ART/settings.py" \
  -D "here=$DMG_ART" -D "app=$APP" -D "readme=$ROOT/assets/platforms/macos/README.txt" \
  -D "icns=$ROOT/assets/icons/app/macos/exports/icon.icns" \
  Intcad "$DMG"

# Check the image as a user gets it: mount it and look inside.
MNT="$(mktemp -d)"
hdiutil attach -readonly -nobrowse -mountpoint "$MNT" "$DMG"
ls -la "$MNT" "$MNT/Intcad.app/Contents" "$MNT/Intcad.app/Contents/MacOS" "$MNT/Intcad.app/Contents/Resources"
test -L "$MNT/Applications" && test "$(readlink "$MNT/Applications")" = /Applications
# The window's look: Finder layout, background art and the volume icon.
test -s "$MNT/.DS_Store" && test -s "$MNT/.VolumeIcon.icns"
test -s "$MNT/.background.tiff"
plutil -lint "$MNT/Intcad.app/Contents/Info.plist"
# Check the names Finder/LaunchServices actually use, and the bundle identity. The executable and
# icon file keep the upstream names so existing bundles and update scripts stay compatible.
for key in CFBundleName CFBundleDisplayName; do
  test "$(/usr/libexec/PlistBuddy -c "Print :$key" "$MNT/Intcad.app/Contents/Info.plist")" = Intcad
done
for key in CFBundleExecutable CFBundleIconFile; do
  test "$(/usr/libexec/PlistBuddy -c "Print :$key" "$MNT/Intcad.app/Contents/Info.plist")" = Intcad
done
# Nothing names or carries a layered icon, which macOS 26 would show in place of the icon file.
if /usr/libexec/PlistBuddy -c 'Print :CFBundleIconName' "$MNT/Intcad.app/Contents/Info.plist" >/dev/null 2>&1; then
  echo "the bundle names a layered icon (CFBundleIconName)" >&2
  exit 1
fi
test ! -e "$MNT/Intcad.app/Contents/Resources/Assets.car"
test "$(/usr/libexec/PlistBuddy -c 'Print :CFBundleIdentifier' "$MNT/Intcad.app/Contents/Info.plist")" = "$MACOS_BUNDLE_ID"
test -s "$MNT/Intcad.app/Contents/Resources/Intcad.icns"
cmp "$ROOT/assets/icons/app/macos/exports/icon.icns" "$MNT/Intcad.app/Contents/Resources/Intcad.icns"
cmp "$ROOT/assets/platforms/macos/README.txt" "$MNT/README.txt"
case "$ARCH" in universal) WANT=(arm64 x86_64) ;; aarch64) WANT=(arm64) ;; *) WANT=(x86_64) ;; esac
for arch in "${WANT[@]}"; do
  lipo "$MNT/Intcad.app/Contents/MacOS/Intcad" -verify_arch "$arch"
done
test -s "$MNT/Intcad.app/Contents/Resources/Licenses/THIRD-PARTY-LICENSES.txt"
cmp "$ROOT/packages/assistant-web/THIRD-PARTY-NOTICES.txt" "$MNT/Intcad.app/Contents/Resources/Licenses/ASSISTANT-THIRD-PARTY-NOTICES.txt"
cmp "$ROOT/packaging/notices/WEB-VIEW-PLATFORM-NOTICES.txt" "$MNT/Intcad.app/Contents/Resources/Licenses/WEB-VIEW-PLATFORM-NOTICES.txt"
# The page's files are embedded in the executable and its inventory/notices are beside it: check
# the app as the mounted image carries it, not the build output it came from.
python3 "$ROOT/packaging/macos/verify-assistant-bundle.py" --app "$MNT/Intcad.app" --source "$ROOT/packages/assistant-web"
if [ "$WITH_MASTRA" = 1 ]; then
  case "$ARCH" in
    universal)
      test -f "$MNT/Intcad.app/Contents/Resources/mastra/aarch64-apple-darwin/runtime.json"
      test -f "$MNT/Intcad.app/Contents/Resources/mastra/x86_64-apple-darwin/runtime.json"
      ;;
    *) test -f "$MNT/Intcad.app/Contents/Resources/mastra/runtime.json" ;;
  esac
fi
if [ "$WITH_MASTRA_AGENT" = 1 ]; then
  # The agent's folder as the image carries it, checked again, and run once against stand-ins.
  node "$ROOT/packages/mastra-runtime/scripts/agent-package.mjs" check --read-only --target "${TARGETS[0]}" "$MNT/Intcad.app/Contents/Resources/mastra-agent"
  node "$ROOT/packages/mastra-runtime/scripts/agent-smoke.mjs" "$MNT/Intcad.app/Contents/Resources/mastra-agent"
fi
# Not --strict: the mounted HFS volume adds Finder info, which strict verification rejects.
codesign --verify --deep --verbose=2 "$MNT/Intcad.app"
"$MNT/Intcad.app/Contents/MacOS/Intcad" --version
# Programs were just run from the image (with the built-in agent, its Node and what it started):
# the system may hold the volume for a moment after they end.
detached=0
for _ in 1 2 3 4 5; do
  if hdiutil detach "$MNT"; then detached=1; break; fi
  sleep 2
done
if [ "$detached" = 0 ]; then hdiutil detach "$MNT" -force; fi

if [ -n "$SIGN_IDENTITY" ]; then
  sign_code "$DMG"
  codesign --verify --verbose=2 "$DMG"
fi
if [ "$NOTARIZE" = 1 ]; then
  echo "==> notarizing $DMG"
  notarize "$DMG"
  staple "$DMG"
  xcrun stapler validate "$DMG"
  # Gatekeeper's verdict on the DMG and on the app as a user gets it.
  spctl -a -t open --context context:primary-signature -vv "$DMG"
  MNT="$(mktemp -d)"
  hdiutil attach -readonly -nobrowse -mountpoint "$MNT" "$DMG"
  spctl --assess --type execute --verbose=4 "$MNT/Intcad.app"
  hdiutil detach "$MNT"
fi

# ---- CLI ---------------------------------------------------------------------------------------
echo "==> building $CLI_ZIP"
CLI_DIR="$WORK/intcad-cli-$VERSION-macos-$ARCH"
mkdir -p "$CLI_DIR"
cp "$WORK/bin/intcad-cli" "$CLI_DIR/"
sign_code "$CLI_DIR/intcad-cli" "$MACOS_BUNDLE_ID.cli"
copy_docs "$CLI_DIR"
rm -f "$CLI_ZIP"
ditto -c -k --keepParent "$CLI_DIR" "$CLI_ZIP"
if [ "$NOTARIZE" = 1 ]; then
  # A zip of bare executables can't be stapled; Gatekeeper finds the ticket online.
  echo "==> notarizing $CLI_ZIP"
  notarize "$CLI_ZIP"
fi

"$WORK/bin/intcad-cli" --version
echo "==> done"
ls -lh "$DMG" "$CLI_ZIP"
