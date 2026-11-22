#!/usr/bin/env bash
# Build and package SolidCraft for Linux (<arch> is x86_64 or aarch64):
#
#   $DIST/intcad-<version>-linux-<arch>.AppImage        any distro with glibc >= the build host's
#   $DIST/intcad-<version>-linux-<arch>.AppImage.zsync  delta updates (needs zsyncmake)
#   $DIST/{solidcraft,solvecraft}-<version>-linux-<arch>.AppImage.zsync  the same, for AppImages
#                                                        published under those earlier names
#   $DIST/intcad-<version>-linux-<arch>.deb             Debian, Ubuntu, Mint, Pop!_OS, ...
#   $DIST/intcad-<version>-linux-<arch>.rpm             Fedora, openSUSE, RHEL, ...
#   $DIST/intcad-<version>-linux-<arch>.tar.gz          plain FHS-style tree (bin/, share/)
#
# Usage: packaging/linux/package.sh [--skip-build] [--formats "appimage deb rpm tar"] [--with-mastra] [--with-mastra-agent|--without-mastra-agent]
#
# Needs: cargo; nfpm for deb/rpm (https://nfpm.goreleaser.com); appimagetool for the AppImage
# (downloaded into $CARGO_TARGET_DIR if missing). Build on an old distro (CI: Ubuntu 22.04,
# glibc 2.35) so the binaries run on newer ones. Optional: desktop-file-validate, appstreamcli,
# zsyncmake (the zsync package) for the AppImage's .zsync.
# Adapted from PhotoCraft's packaging (storytold/photocraft, Apache-2.0; see ATTRIBUTION.md).
set -euo pipefail
# shellcheck source=../env.sh
. "$(dirname "${BASH_SOURCE[0]}")/../env.sh"
HERE="$ROOT/packaging/linux"

SKIP_BUILD=0
FORMATS="appimage deb rpm tar"
WITH_MASTRA="${SOLIDCRAFT_WITH_MASTRA:-0}"
WITH_MASTRA_AGENT="${SOLIDCRAFT_WITH_MASTRA_AGENT:-0}"
while [ $# -gt 0 ]; do
  case "$1" in
    --skip-build) SKIP_BUILD=1; shift ;;
    --formats) FORMATS="$2"; shift 2 ;;
    --with-mastra) WITH_MASTRA=1; shift ;;
    --without-mastra) WITH_MASTRA=0; shift ;;
    --with-mastra-agent) WITH_MASTRA_AGENT=1; shift ;;
    --without-mastra-agent) WITH_MASTRA_AGENT=0; shift ;;
    -h | --help) sed -n '2,16p' "$0"; exit 0 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done

ARCH="${CROSS_ARCH:-$(uname -m)}"
case "$ARCH" in
  riscv64) DEB_ARCH=riscv64 ;;
  x86_64) DEB_ARCH=amd64 ;;
  aarch64 | arm64) ARCH=aarch64; DEB_ARCH=arm64 ;;
  *) echo "unsupported architecture $ARCH" >&2; exit 2 ;;
esac
export SOLIDCRAFT_MAINTAINER="${SOLIDCRAFT_MAINTAINER:-${SOLVECRAFT_MAINTAINER:-Intgral <https://github.com/intgral-ai/intcad>}}"  # SOLVECRAFT_*: compatibility shim, see docs/compatibility.md
BASENAME="intcad-$VERSION-linux-$ARCH"

echo "==> SolidCraft $VERSION for Linux $ARCH ($FORMATS)"

if [ "$SKIP_BUILD" = 0 ]; then
  if [ -n "${CROSS_TARGET:-}" ]; then
    (cd "$ROOT" && cargo build --release --locked -p solidcraft -p solidcraft-cli --target "$CROSS_TARGET")
    BIN="$CARGO_TARGET_DIR/$CROSS_TARGET/release"
  else
    (cd "$ROOT" && cargo build --release --locked -p solidcraft -p solidcraft-cli)
    BIN="$CARGO_TARGET_DIR/release"
  fi
else
  if [ -n "${CROSS_TARGET:-}" ]; then
    BIN="$CARGO_TARGET_DIR/$CROSS_TARGET/release"
  else
    BIN="$CARGO_TARGET_DIR/release"
  fi
fi
WORK="$CARGO_TARGET_DIR/linux-package"
STAGE="$WORK/root"
# An earlier run may have left the built-in agent's folder, which nothing may write.
if [ -d "$WORK" ]; then chmod -R u+w "$WORK"; fi
rm -rf "$WORK"

# ---- stage an FHS tree (shared by every format) -------------------------------------------------
install -Dm755 "$BIN/intcad" "$STAGE/usr/bin/intcad"
install -Dm755 "$BIN/intcad-cli" "$STAGE/usr/bin/intcad-cli"
strip "$STAGE/usr/bin/intcad" "$STAGE/usr/bin/intcad-cli" 2>/dev/null || true
install -Dm644 "$ROOT/assets/platforms/linux/$APP_ID.desktop" "$STAGE/usr/share/applications/$APP_ID.desktop"
install -Dm644 "$ROOT/assets/platforms/linux/$APP_ID.mime.xml" "$STAGE/usr/share/mime/packages/$APP_ID.xml"
mkdir -p "$STAGE/usr/share/metainfo"
sed -e "s/@VERSION@/$VERSION/g" -e "s/@DATE@/$SOLIDCRAFT_BUILD_DATE/g" \
  "$ROOT/assets/platforms/linux/$APP_ID.metainfo.xml.in" >"$STAGE/usr/share/metainfo/$APP_ID.metainfo.xml"
make_icons "$STAGE/usr/bin/intcad-cli" "$WORK/icons"
mkdir -p "$STAGE/usr/share/icons"
cp -R "$WORK/icons/hicolor" "$STAGE/usr/share/icons/"
mkdir -p "$STAGE/usr/share/doc/intcad"
copy_docs "$STAGE/usr/share/doc/intcad"
# The third-party notices of the Assistant's page (Mastra's components, npm packages, fonts),
# which the app carries built in as well (Help > About > Licences).
install -Dm644 "$ROOT/packages/assistant-web/THIRD-PARTY-NOTICES.txt" "$STAGE/usr/share/doc/intcad/ASSISTANT-THIRD-PARTY-NOTICES.txt"
# The platform web view's licences (GTK 3, WebKitGTK 4.1, WebView2, WKWebView), including the
# LGPL text a dynamically linked system library needs. Also built into About > Licences.
install -Dm644 "$ROOT/packaging/notices/WEB-VIEW-PLATFORM-NOTICES.txt" "$STAGE/usr/share/doc/intcad/WEB-VIEW-PLATFORM-NOTICES.txt"

if command -v desktop-file-validate >/dev/null; then
  desktop-file-validate "$STAGE/usr/share/applications/$APP_ID.desktop"
fi
if command -v appstreamcli >/dev/null; then
  appstreamcli validate --no-net --explain "$STAGE/usr/share/metainfo/$APP_ID.metainfo.xml"
fi

# ---- Mastra runtime (opt-in; application-relative, pinned Node) ---------------------------------
# Bundling is opt-in because the artifact is still unapproved (DQ-7, #15). Ordinary packaging
# keeps the previous release behavior; --with-mastra (or SOLIDCRAFT_WITH_MASTRA=1) asks for the
# workflow and then fails closed until the owner approves a reviewed artifact.
if [ "$WITH_MASTRA" = 1 ]; then
  case "$ARCH" in
    x86_64)
      "$HERE/../mastra/stage.sh" --target x86_64-unknown-linux-gnu --dest "$STAGE/usr/lib/intcad/mastra"
      mkdir -p "$STAGE/usr/share/doc/intcad"
      cp "$STAGE/usr/lib/intcad/mastra/licenses/LICENSES.txt" "$STAGE/usr/share/doc/intcad/MASTRA-LICENSES.txt"
      cp "$STAGE/usr/lib/intcad/mastra/notices/NOTICE.txt" "$STAGE/usr/share/doc/intcad/MASTRA-NOTICE.txt"
      ;;
    *) echo "error: the built-in Mastra workflow does not ship for Linux $ARCH in 1.0; drop --with-mastra for this package" >&2; exit 2 ;;
  esac
else
  mkdir -p "$STAGE/usr/lib/intcad"
  warn "the built-in Mastra workflow is not bundled (opt-in; the runtime artifact is unapproved, DQ-7/#15)"
fi

# ---- Built-in agent (the release's default) -----------------------------------------------------
# Its own folder beside `mastra/`: the app starts the Node in it with agent/main.mjs, and looks for
# it in the package before anything else. stage-agent.sh checks the folder before copying it, the
# staged tree is looked through for anything like a secret or a stored sign-in, and the agent is
# then started from the stage against stand-ins. `--without-mastra-agent` leaves it out.
#
# The agent is built for x86_64 Linux only. A package for another CPU carries none and says so;
# it must not declare one the app could not run.
AGENT_TARGET=x86_64-unknown-linux-gnu
if [ "$WITH_MASTRA_AGENT" = 1 ]; then
  case "$ARCH" in
    x86_64)
      "$HERE/../mastra/stage-agent.sh" --target "$AGENT_TARGET" --dest "$STAGE/usr/lib/intcad/mastra-agent"
      install -Dm644 "$STAGE/usr/lib/intcad/mastra-agent/licenses/THIRD-PARTY-LICENSES.txt" "$STAGE/usr/share/doc/intcad/MASTRA-AGENT-THIRD-PARTY-LICENSES.txt"
      install -Dm644 "$STAGE/usr/lib/intcad/mastra-agent/licenses/NODE-LICENSE.txt" "$STAGE/usr/share/doc/intcad/MASTRA-AGENT-NODE-LICENSE.txt"
      # Nothing in the folder may be written once it is installed: the agent keeps what it
      # writes in the user's data folder. Every format carries these modes.
      chmod -R a-w "$STAGE/usr/lib/intcad/mastra-agent"
      node "$ROOT/packages/mastra-runtime/scripts/agent-package.mjs" check --read-only --target "$AGENT_TARGET" "$STAGE/usr/lib/intcad/mastra-agent"
      node "$ROOT/packages/mastra-runtime/scripts/agent-package.mjs" scan "$STAGE"
      # The agent as the packages will carry it, started and answering against stand-ins.
      node "$ROOT/packages/mastra-runtime/scripts/agent-smoke.mjs" "$STAGE/usr/lib/intcad/mastra-agent"
      ;;
    *) echo "error: the built-in agent's folder is built for Linux x86_64 only; pass --without-mastra-agent for Linux $ARCH" >&2; exit 2 ;;
  esac
else
  warn "the built-in agent is not bundled (--without-mastra-agent); a release package carries it"
fi

has() { case " $FORMATS " in *" $1 "*) return 0 ;; *) return 1 ;; esac; }

# ---- .tar.gz ------------------------------------------------------------------------------------
if has tar; then
  mkdir -p "$WORK/tar"
  cp -R "$STAGE/usr" "$WORK/tar/$BASENAME"
  tar -C "$WORK/tar" -czf "$DIST/$BASENAME.tar.gz" "$BASENAME"
  echo "wrote $DIST/$BASENAME.tar.gz"
fi

# ---- .deb / .rpm --------------------------------------------------------------------------------
if has deb || has rpm; then
  command -v nfpm >/dev/null || { echo "error: nfpm not found (https://nfpm.goreleaser.com/install/)" >&2; exit 1; }
  export VERSION
  export NFPM_ARCH="$DEB_ARCH"
  # nfpm expands env vars in fields like `version` and `arch`, but not in `contents[].src`.
  sed "s|\${STAGE}|$STAGE|g" "$HERE/nfpm.yaml" >"$WORK/nfpm.yaml"
  for fmt in deb rpm; do
    if has "$fmt"; then (cd "$ROOT" && nfpm package -f "$WORK/nfpm.yaml" -p "$fmt" -t "$DIST/$BASENAME.$fmt"); fi
  done
fi

# ---- AppImage -----------------------------------------------------------------------------------
if has appimage; then
  APPDIR="$WORK/SolidCraft.AppDir"
  cp -R "$STAGE" "$APPDIR"
  mv "$APPDIR/usr/share/doc" "$WORK/doc-unused"
  # AppRun is a script, not a symlink: it installs the .desktop entry and icons into
  # $XDG_DATA_HOME before launching, so Wayland compositors can resolve the window's app_id to
  # our icon.
  install -Dm755 "$HERE/AppRun" "$APPDIR/AppRun"
  cp "$ROOT/assets/platforms/linux/$APP_ID.desktop" "$APPDIR/$APP_ID.desktop"
  cp "$WORK/icons/hicolor/256x256/apps/$APP_ID.png" "$APPDIR/$APP_ID.png"
  ln -s "$APP_ID.png" "$APPDIR/.DirIcon"

  TOOL="${APPIMAGETOOL:-$(command -v appimagetool || true)}"
  if [ -z "$TOOL" ]; then
    TOOL="$CARGO_TARGET_DIR/appimagetool-$ARCH.AppImage"
    if [ ! -x "$TOOL" ]; then
      curl -fsSL -o "$TOOL" "https://github.com/AppImage/appimagetool/releases/download/continuous/appimagetool-$ARCH.AppImage"
      chmod +x "$TOOL"
    fi
  fi
  OUT="$DIST/$BASENAME.AppImage"
  APPDIR="$(cd "$APPDIR" && pwd)"
  TOOL="$(cd "$(dirname "$TOOL")" && pwd)/$(basename "$TOOL")"
  rm -f "$OUT.zsync"
  # Update information: AppImageUpdate and the like fetch only the changed blocks of a newer
  # release through the .zsync published next to each AppImage. `latest` is the newest published
  # release that is not a pre-release.
  #
  # It is embedded in every AppImage and read by an anonymous AppImageUpdate, so it must name the
  # public distribution repository. Never derive it from `GITHUB_REPOSITORY`: a package built in
  # the private source repository would then tell every user's updater to fetch from a repository
  # they cannot read.
  DIST_REPO="${SOLIDCRAFT_DIST_REPO:-intgral-ai/intcad-releases}"
  UPDATE_INFO="gh-releases-zsync|${DIST_REPO%%/*}|${DIST_REPO#*/}|latest|intcad-*-linux-$ARCH.AppImage.zsync"
  # Extract-and-run: works without FUSE (containers, CI). With zsyncmake on the host appimagetool
  # also writes the .zsync, into its working directory, hence the cd.
  (cd "$DIST" && ARCH="$ARCH" APPIMAGE_EXTRACT_AND_RUN=1 "$TOOL" --no-appstream -u "$UPDATE_INFO" "$APPDIR" "$OUT")
  echo "wrote $OUT"
  if [ -s "$OUT.zsync" ]; then
    echo "wrote $OUT.zsync"
    # Compatibility shim (docs/compatibility.md): an AppImage published under an earlier name
    # looks in the latest release for the .zsync under that name: `solidcraft` until Intcad had
    # its own, `solvecraft` before the rename. The same .zsync under each name (it names the new
    # AppImage's file, relative to the release) updates them to this one.
    for earlier in solidcraft solvecraft; do
      alias="$DIST/$earlier-$VERSION-linux-$ARCH.AppImage.zsync"
      cp "$OUT.zsync" "$alias"
      # The earlier build's selector is <name>-*-linux-<arch>.AppImage.zsync: the copy must
      # match it and lead to the new image.
      grep -a -m1 -qxF "Filename: $BASENAME.AppImage" "$alias" || { echo "error: $alias does not name $BASENAME.AppImage" >&2; exit 1; }
      echo "wrote $alias (update alias for the earlier name)"
    done
  else
    warn "zsyncmake not found, so $OUT.zsync was not written; AppImage delta updates need it"
  fi
fi

"$STAGE/usr/bin/intcad-cli" --version
echo "==> done"
ls -lh "$DIST"
