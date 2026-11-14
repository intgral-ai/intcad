{
  description = "SolidCraft — native app development shell and package for Linux and macOS";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs =
    { self, nixpkgs }:
    let
      systems = [
        "x86_64-linux"
        "aarch64-linux"
        "aarch64-darwin"
      ];
      forAllSystems = f: nixpkgs.lib.genAttrs systems (system: f nixpkgs.legacyPackages.${system});

      # Linux only. Why this shell exists. winit opens the window by dlopen'ing the Wayland client libraries,
      # wgpu renders through the EGL loader (libEGL.so.1), and rfd's file dialogs reach
      # xdg-desktop-portal through libdbus-1; a plain `cargo run` under NixOS finds none of them,
      # because there is no /usr/lib and nothing puts them on LD_LIBRARY_PATH:
      #   winit EventLoopError: The wayland library could not be loaded
      #   eframe: WGPU error: Failed to create surface for any enabled backend: {}
      #   rfd::backend::xdg_desktop_portal::portal::libdbus: Can't connect to a portal: libdbus-1.so not found
      # The Mesa drivers themselves — and the EGL vendor / Vulkan ICD descriptions that name them —
      # come from the system in /run/opengl-driver (hardware.graphics.enable), not from here, since
      # that is the driver stack the compositor runs on. Verified on an Intel LNL iGPU under
      # Wayland: wgpu picks the GL backend through this loader and draws.
      # The X11 libraries are the ones the app actually uses on a Wayland desktop: with `DISPLAY`
      # set and `SOLIDCRAFT_WAYLAND` unset, winit runs through XWayland (file drag and drop exists
      # only there). The Wayland ones are for `SOLIDCRAFT_WAYLAND=1`, and both are needed under
      # `xvfb-run` on a machine with no display at all.
      runtimeLibs =
        pkgs:
        pkgs.lib.optionals pkgs.stdenv.isLinux (
          (with pkgs; [
            wayland
            libxkbcommon
            libglvnd
            vulkan-loader
            dbus
          ])
          ++ (with pkgs; [
            # xorg.libX11 and friends are deprecated aliases in current nixpkgs.
            libx11
            libxcursor
            libxi
            libxrandr
            libxrender
            libxext
            libxcb
          ])
          # The Assistant is drawn by a WebKitGTK page in a GTK child window. These are linked
          # directly, so they are both build inputs and runtime closure members: the packages
          # and the wrapper must carry them, not leave them to the host distribution.
          ++ (with pkgs; [
            gtk3
            webkitgtk_4_1
          ])
          # A fcitx5 GTK module built against this GTK/GLib. A module from another
          # nixpkgs/system can fail GTK's GLib micro-version check; the wrapper puts a
          # matching one first, without choosing an input method for the user.
          ++ pkgs.lib.optionals pkgs.stdenv.isLinux [ pkgs.fcitx5-gtk ]
        );
      # macOS needs none of the above: winit talks to AppKit, wgpu renders through Metal, and the
      # OS ships its GPU drivers, so the darwin package and shells skip the wrapper and paths.
      driverLibDir = pkgs: if pkgs.stdenv.isLinux then "/run/opengl-driver/lib" else "";
      driverShareDir = pkgs: if pkgs.stdenv.isLinux then "/run/opengl-driver/share" else "";
      libraryPath =
        pkgs:
        if pkgs.stdenv.isLinux then
          "${pkgs.lib.makeLibraryPath (runtimeLibs pkgs)}:${driverLibDir pkgs}"
        else
          "";

      shellHook = pkgs: ''
        ${pkgs.lib.optionalString pkgs.stdenv.isLinux ''
          if [ ! -d ${driverLibDir pkgs} ]; then
            echo "SolidCraft: ${driverLibDir pkgs} is missing — on NixOS enable hardware.graphics; on other distributions use a matching nixGL driver wrapper." >&2
          fi
        ''}
        command -v cc >/dev/null || echo "solidcraft: no C linker on PATH, cargo cannot link — use nix develop .#rust" >&2
        echo "solidcraft: cargo run --release -p solidcraft -- --sample"
        ${pkgs.lib.optionalString pkgs.stdenv.isLinux ''
          export XDG_DATA_DIRS="${driverShareDir pkgs}:''${XDG_DATA_DIRS:-/usr/local/share:/usr/share}"
        ''}
      '';
    in
    {
      devShells = forAllSystems (pkgs: {
        # The shell to work in. It brings no toolchain of its own, Rust or C: the ones already on
        # PATH (a system package, rustup, nixpkgs) keep their build cache and their linker, where a
        # second rustc — or mkShell's second gcc, with another glibc behind it — would make every
        # `cargo build` produce binaries from somewhere else.
        default = pkgs.mkShellNoCC {
          name = "solidcraft";
          nativeBuildInputs = [ pkgs.pkg-config ];
          buildInputs = runtimeLibs pkgs;
          LD_LIBRARY_PATH = libraryPath pkgs;
          shellHook = shellHook pkgs;
        };

        # For a machine with no Rust at all: `nix develop .#rust`. Its own target directory, so
        # switching between this toolchain and the system one does not thrash the cache.
        rust = pkgs.mkShell {
          name = "solidcraft-rust";
          packages = [
            pkgs.cargo
            pkgs.rustc
            pkgs.rustfmt
            pkgs.clippy
            pkgs.pkg-config
          ];
          buildInputs = runtimeLibs pkgs;
          LD_LIBRARY_PATH = libraryPath pkgs;
          # Under target/, which .gitignore already covers.
          CARGO_TARGET_DIR = "target/nix";
          shellHook = ''
            ${shellHook pkgs}
            echo "solidcraft: $(rustc --version), building into target/nix"
          '';
        };
      });

      packages = forAllSystems (pkgs: rec {
        default = solidcraft;
        solidcraft =
          assert pkgs.lib.assertMsg
            (pkgs.lib.versionAtLeast pkgs.rustc.version (pkgs.lib.importTOML ./Cargo.toml)
            .workspace.package.rust-version)
            "SolidCraft requires the Rust version advertised in Cargo.toml; update the pinned nixpkgs input.";
          pkgs.rustPlatform.buildRustPackage {
            pname = "solidcraft";
            version = (pkgs.lib.importTOML ./Cargo.toml).workspace.package.version;
            src = self;

            # The kernel's geometry record is a pinned Git dependency; importCargoLock needs the
            # exact source hash to build without network. The revisions match Cargo.toml.
            cargoLock = {
              lockFile = ./Cargo.lock;
              outputHashes = {
                "solidcore-1.0.1" = "sha256-SziXzxkQXY29PBOG7Ct3W8dex4RUWfAEczFUakB0k50=";
                "solidcore-record-0.1.0" = "sha256-5LUUk4lmC3igIXyHnmJEXj17je0gEH9AoB0iTd5z0A0=";
              };
            };
            # The CLI draws the same packaging icons as the other Linux distributions.
            cargoBuildFlags = [
              "-p"
              "solidcraft"
              "-p"
              "solidcraft-cli"
            ];
            cargoInstallFlags = [
              "-p"
              "solidcraft"
              "-p"
              "solidcraft-cli"
            ];
            # `cargo xtask ci` is the test gate (it wants a wasm target and a GPU); the sandbox has
            # neither.
            doCheck = false;

            nativeBuildInputs = [
              pkgs.makeWrapper
              pkgs.pkg-config
              pkgs.wrapGAppsHook3
            ];
            buildInputs = runtimeLibs pkgs;

            postInstall = ''
              ${pkgs.lib.optionalString pkgs.stdenv.isLinux ''
                install -Dm644 assets/platforms/linux/io.tangencylabs.solidcraft.desktop \
                  $out/share/applications/io.tangencylabs.solidcraft.desktop
                install -Dm644 assets/platforms/linux/io.tangencylabs.solidcraft.mime.xml \
                  $out/share/mime/packages/io.tangencylabs.solidcraft.xml
                install -Dm644 packaging/notices/WEB-VIEW-PLATFORM-NOTICES.txt \
                  $out/share/doc/solidcraft/WEB-VIEW-PLATFORM-NOTICES.txt
                for size in 16 24 32 48 64 128 256 512; do
                  icon=$out/share/icons/hicolor/''${size}x''${size}/apps/io.tangencylabs.solidcraft.png
                  mkdir -p "$(dirname "$icon")"
                  cp assets/icons/app/linux/exports/hicolor/''${size}x''${size}/apps/io.tangencylabs.solidcraft.png "$icon" >/dev/null
                done
              ''}
            '';

            # Linux only: the app reaches the window through dlopen'd libraries, so the packaged
            # binary needs the same LD_LIBRARY_PATH the shell exports. File dialogs go through the
            # desktop portal and fall back to zenity without one (#2177), so zenity comes along,
            # after the system's own on PATH. wrapGAppsHook3 adds the GDK pixbuf loaders, GIO
            # modules and GSettings schemas GTK 3 and WebKitGTK need outside nix develop; without
            # them the Assistant's web view cannot make a page. macOS needs no wrapper.
            preFixup = pkgs.lib.optionalString pkgs.stdenv.isLinux ''
              # GTK finds input methods through its immodules cache. The system one on NixOS names
              # the session's fcitx5-gtk, built against a different GTK/GLib than this package:
              # GTK then refuses it ("GLib version too old (micro mismatch)") and no Chinese input
              # reaches the page. Build a cache from this closure's GTK and fcitx5-gtk and point
              # GTK_IM_MODULE_FILE at it, so the module matches the GTK that loads it.
              mkdir -p $out/lib/gtk-3.0/3.0.0
              GTK_PATH=${pkgs.gtk3}/lib/gtk-3.0:${pkgs.fcitx5-gtk}/lib/gtk-3.0 \
                ${pkgs.gtk3.dev}/bin/gtk-query-immodules-3.0 > $out/lib/gtk-3.0/3.0.0/immodules.cache
              gappsWrapperArgs+=(
                --prefix LD_LIBRARY_PATH : "${libraryPath pkgs}"
                --suffix PATH : "${pkgs.lib.makeBinPath [ pkgs.zenity ]}"
                --set-default XDG_DATA_DIRS /usr/local/share:/usr/share
                --prefix XDG_DATA_DIRS : "$out/share:${driverShareDir pkgs}"
                --prefix GTK_PATH : "${pkgs.fcitx5-gtk}/lib/gtk-3.0"
                --set GTK_IM_MODULE_FILE "$out/lib/gtk-3.0/3.0.0/immodules.cache"
              )
            '';

            meta = {
              description = "Rust-native parametric 3D CAD app";
              homepage = "https://github.com/intgral-ai/open-skyfall-desktop";
              # The app is MIT/Apache-2.0. The closure also carries GTK 3 (LGPL-2.1-or-later) and
              # WebKitGTK 4.1 (LGPL-2.1-or-later and BSD-2-Clause), dynamically linked; their
              # notice text is installed under share/doc/solidcraft.
              license = with pkgs.lib.licenses; [
                mit
                asl20
                lgpl21Plus
                bsd2
              ];
              mainProgram = "solidcraft";
            };
          };
      });

      apps = forAllSystems (
        pkgs:
        let
          package = self.packages.${pkgs.stdenv.hostPlatform.system}.solidcraft;
          desktop = {
            type = "app";
            program = "${package}/bin/solidcraft";
          };
        in
        {
          default = desktop;
          solidcraft = desktop;
          cli = {
            type = "app";
            program = "${package}/bin/solidcraft-cli";
          };
        }
      );
    };
}
