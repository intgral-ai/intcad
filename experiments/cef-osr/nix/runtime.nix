# Runtime libraries CEF's prebuilt Linux binaries need, wrapped as one command.
#
# CEF is not a NixOS system library and its `libcef.so` carries no RPATH into a
# Nix closure, so a probe built with `cargo` finds its own libs but not CEF's.
# `lib.makeLibraryPath` collects every output's `lib/`, which a `symlinkJoin`
# of the same packages does not do reliably (it kept nss but dropped glib here).
#
#   nix build --impure --expr 'import ./runtime.nix' -o /tmp/cef-osr-runtime
#   export CEF_PATH=/path/to/cef_binary_154.0.34_linux64
#   /tmp/cef-osr-runtime/bin/cef-osr-env ./target/release/cef-osr-probe
#
# No patchelf, no FHS environment, no second glibc in the process.
{ pkgs ? import (builtins.getFlake "nixpkgs") { } }:
let
  deps = with pkgs; [
    glib
    nss
    nspr
    libgbm
    libdrm
    libxkbcommon
    cups
    at-spi2-atk
    at-spi2-core
    atk
    pango
    cairo
    gdk-pixbuf
    alsa-lib
    dbus
    expat
    libx11
    libxcb
    libxcomposite
    libxdamage
    libxext
    libxfixes
    libxrandr
    libxshmfence
    libxi
    libxtst
    fontconfig
    freetype
    harfbuzz
    systemd
  ];
in
pkgs.writeShellScriptBin "cef-osr-env" ''
  export LD_LIBRARY_PATH="${pkgs.lib.makeLibraryPath deps}:''${CEF_PATH:-}:''${LD_LIBRARY_PATH:-}"
  exec "$@"
''
