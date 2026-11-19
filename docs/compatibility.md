# Compatibility with the SolveCraft and SolidCraft names

This repository's product is **Intcad**, an Intgral fork. On a user's machine Intcad has an
identity of its own, so an install of it is not an install of upstream SolidCraft under another
label: the app ID, the macOS bundle, the Windows installer identity, the package and release file
names, the settings and data folders, the programs (`intcad`, `intcad-cli`) and the MCP server
entry are Intcad's ([Intcad's identity](#intcads-identity)). Names inside the code remain
`solidcraft` so designs, scripts and merges from upstream keep working: crate and Cargo package
names, `.sc` and the `solidcraft/N` tag and `SOLIDCRAFT_*` variables. The upstream project keeps
the SolidCraft name and logo.

SolidCraft was first called SolveCraft, and builds up to 0.4 kept that name in their
identifiers after the displayed name changed. 0.5.0 renamed the binaries, release assets and
packages; the release after it renames everything else: crates `solidcraft-*`, binaries `solidcraft` and `solidcraft-cli`, `.solidcraft`
designs (saved as `.sc` since; see section 1), the `"format": "solidcraft/N"` tag, `SOLIDCRAFT_*` environment variables, settings
and recovery folders, the MCP server name, docs, CI and packaging.

The old name survives **only** in the shims below (plus their tests, this file, and changelog /
release notes). `git grep -i solvecraft` must list nothing else. When you add or remove a
shim, update this file in the same PR.

## 1. `.solvecraft` and `.solidcraft` design files

New designs are saved as `.sc`. Files with either older extension still open everywhere: `.solvecraft`
(saved before the rename) and `.solidcraft` (saved before designs moved to `.sc`). One constant
names the extension new designs get, `DESIGN_EXT` (`sc`), and one list names every extension a
design may have, `DESIGN_EXTS` (`sc`, then `LEGACY_DESIGN_EXTS`: `solidcraft`, `solvecraft`);
code uses those, never its own string.

| Path in | Where |
|---|---|
| Extension → design format | `crates/io/src/lib.rs` (`DESIGN_EXTS`, `is_design_ext`, `is_design_path`, format detection) |
| Open dialog filters (desktop and browser) | `crates/io/src/formats.rs` (the design row lists every entry of `DESIGN_EXTS`), shown by `apps/solidcraft/src/main.rs` (`services`) |
| Drag-drop, recent files, `doc.open`, MCP `open`, CLI `eval`/`run`, Insert/Link/Relink design | go through the same format detection in `solidcraft_io` (`is_design_container_path` for links) |
| Browser file picker | `apps/solidcraft-web/src/disk.rs` (`open_options`, `opens_design`) |
| Windows | `assets/platforms/windows/solidcraft.wxs`: `.sc` is the design ProgId's extension; `.solidcraft` and `.solvecraft` point at the same ProgId and are in `SupportedTypes` |
| macOS | `assets/platforms/macos/Info.plist.in`: `sc`, `solidcraft` and `solvecraft` in the design document type and its `io.tangencylabs.solidcraft.design` UTI |
| Linux / FreeBSD / Flatpak / Snap | `assets/platforms/linux/io.tangencylabs.solidcraft.mime.xml`: `*.sc`, `*.solidcraft` and `*.solvecraft` globs under `application/x-solidcraft` (plus the `application/x-solvecraft` alias); the `.desktop` file names that type |

Opening a `.solidcraft` file and pressing **Save** writes that same file: nothing is renamed behind
the user's back. **Save As** (and every new design, autosave, crash report and component
extracted to its own design) offers `.sc`, and leaves the old file as it was.

Recent-files entries that point at an older file keep working: they are plain paths and
open through the same detection. Autosaves and crash reports that earlier releases wrote
(`<id>.solidcraft`, `<id>.solvecraft`, `design.solidcraft`) are still listed and recovered
(`solidcraft_engine::recovery`, `solidcraft_engine::crash::read`), and the browser app still
recovers `recovery.solidcraft` / `recovery.solvecraft` from its storage. The browser app also
lists designs it stored under the old extensions, and keyboard-shortcut files exported as
`solvecraft-shortcuts/1` still import (`crates/ui-egui/src/toolbar_custom.rs`). The designs in
`apps/solidcraft-cli/tests/corpus` keep their `.solidcraft` names on purpose: they are files saved
by earlier releases. Tests: `crates/io` (`Format::from_name`, `is_design_path`, the import filter
groups), `legacy_extension_and_format_tag_still_open` (CLI, `run_results.rs`),
`legacy_design_files_and_resource_uris` (MCP `open` and `save`),
`saving_an_opened_legacy_design_keeps_its_name` (engine),
`autosave_from_a_legacy_folder_is_listed_and_loads`, `autosaves_are_written_as_sc` and
`a_report_from_an_earlier_release_still_finds_its_design`.

## 2. The `solvecraft/N` format tag

Designs record their format as `"format": "solidcraft/N"`. Files saved by builds up to 0.5 say
`"format": "solvecraft/N"`; `crates/doc/src/format.rs` (`version_of`) reads both prefixes and
upgrades them as usual (the file is re-tagged `solidcraft/N` when saved).

Not backward-compatible: builds up to 0.5 don't know the new tag and refuse files saved by
later builds. Test: `version_of` accepts both prefixes; legacy-tagged designs load and are
re-tagged on save.

The current format is 58: a new design is written as `solidcraft/58` (a drawing keeps its own
`solidcraft-drawing/N`). There is one persistence implementation, `solidcraft_format` (the
format `Family` and its `container`), and `crates/doc`/`crates/io` supply only the document types
and the limits. A build that reads up to 51 cannot open a 58 file; a build newer than the file
opens it instead of refusing it: what it reads is read, the parts it can't read (a feature, a
parameter, a section) are named and kept as they came, and the file is saved under its own format
number and change list. Intcad's five pre-58 pending changes (`bridged-counterbore`, `fillet-turn`,
`hole-direction-own-faces`, `revolve-axis-origin-expressions`, `transferred-profile-frame`) are
the files upstream folded into `v51`, byte for byte, so a design any earlier Intcad release saved
is a prefix subset of `v51` and needs no migration: it opens and upgrades to 58 as usual. Tests:
`a_design_saved_at_format_51_with_its_pending_changes_saves_and_reopens` (io), the format and
newer-file tests in `crates/doc/src/format.rs` and `crates/doc/src/newer_file_tests.rs`.

## 3. `SOLVECRAFT_*` environment variables

Every runtime variable is read through one helper, `solidcraft_geom::env` (`var`, `var_os`):
it reads `SOLIDCRAFT_<NAME>` and falls back to `SOLVECRAFT_<NAME>` when the new one is unset.
The shell entry points do the same: `packaging/linux/AppRun`
(`SOLIDCRAFT_NO_DESKTOP_INTEGRATION`) and `packaging/env.sh` (`SOLIDCRAFT_VERSION`,
`SOLIDCRAFT_BUILD_DATE`). Tests: the helper's unit tests (new wins, old is the fallback,
neither → unset).

## 4. Settings, preferences, autosave and recovery folders

New folders use the Intcad name:

| | Linux | macOS | Windows |
|---|---|---|---|
| Preferences, graphics settings | `~/.local/share/intcad` | `~/Library/Application Support/Intcad` | `%APPDATA%\Intcad\data` |
| Recovery (autosave) | `…/intcad/recovery` | `…/Intcad/recovery` | `%LOCALAPPDATA%\Intcad\recovery` |

On first launch, if the new folder doesn't exist but the pre-rename one (`solvecraft` /
`SolveCraft`) does, it is **copied** to the new location; the old folder is never deleted or
changed. The `solidcraft` / `SolidCraft` folder is not a source. It may be there only because
upstream SolidCraft is installed, and Intcad starts clean beside it: upstream's settings,
autosaves and control tokens are neither copied nor offered for recovery (tests
`first_launch_leaves_an_upstream_solidcraft_folder_alone`,
`an_upstream_solidcraft_autosave_is_not_adopted`). Builds of Intcad from before it had folders
of its own used that folder too; their settings are not carried over. The
window and UI state that eframe kept under `ai.storyteller.solvecraft` is copied the same way.
Autosaves copied from the old folder (`<id>.solvecraft`) are listed and recovered like new ones.
The preferences key in eframe's storage is `solidcraft.prefs`; `solvecraft.prefs` is read when
it is missing (desktop and browser, `solidcraft_ui_egui::prefs::stored`). The browser app also
reads files and the autosave it kept under the old names in local storage and OPFS
(`apps/solidcraft-web/src/files.rs`). A 3DxWare profile the driver made for the old executable
name is read until it makes a new one (`apps/solidcraft/src/space_mouse.rs`).
In the Flatpak sandbox these folders live under `~/.var/app/ai.intgral.intcad/data`. Intcad has
no earlier Flatpak app id of its own and reads no folder under an upstream app id (see
[Linux app ID](#linux-app-id)).
The thumbnail cache is not migrated; it is rebuilt. The copy helper is
`solidcraft_io::adopt_legacy_dir`, used by the desktop app (`adopt_legacy_settings`) and
`solidcraft_engine::recovery::default_dir`. Tests: `legacy_dir` (copy once, never overwrite,
old folder intact), `first_launch_adopts_the_previous_release_settings`,
`autosave_from_a_legacy_folder_is_listed_and_loads`, `recovery_dirs_and_their_legacy_names`,
`stored_prefs_fall_back_to_the_legacy_key`.

## 5. `solvecraft-cli` alias

Upstream ships a `solvecraft-cli` that forwards to its CLI. Intcad does not: the name is
upstream's, Intcad was never published under it, and two packages cannot both install
`/usr/bin/solvecraft-cli`. The alias binary and its test are gone from this repository.

Intcad's own programs have no alias for the names they had in 0.1.0 (`solidcraft`,
`solidcraft-cli`) either: those are upstream's names.

## 6. MCP

`serverInfo.name` is `intcad`, and `intcad-cli mcp setup` writes the entries `intcad` and
`intcad-live`. It reads and changes only those two. An entry named `solidcraft` is upstream's or
the user's and stays as it is (test `an_upstream_solidcraft_entry_is_left_alone` in
`crates/engine/src/agent_setup.rs`). New setups use:

```sh
claude mcp add intcad -- intcad-cli mcp
```

Resources are listed as `solidcraft://design` and `solidcraft://commands`; the old
`solvecraft://` URIs still read (`crates/mcp/src/server.rs`, test
`legacy_design_files_and_resource_uris`).

## Intcad's identity

Intcad's own names, set once, in the version after 0.1.0. `scripts/identity_to_intcad.py` applies
the mechanical part and can be run again after a merge from upstream. Intcad 0.1.0 was published
under upstream's names, so an install of it is treated like an upstream one: not replaced, and
nothing is read from its folders.

| What | Intcad | Why it is not upstream's |
|---|---|---|
| App ID (Flatpak, desktop entry, AppStream, MIME and icon file names, Wayland `app_id` / X11 class, Arch package, macOS bundle ID and document type) | `ai.intgral.intcad` (the CLI's signing identifier is `ai.intgral.intcad.cli`) | Upstream's `io.tangencylabs.solidcraft` belongs to upstream. Sharing it made the two apps one app to the desktop, to macOS privacy permissions and to a code signature. |
| macOS bundle | `Intcad.app` | Dragging it to Applications no longer replaces an upstream `SolidCraft.app`. |
| MSI `UpgradeCode`, install folder, registry key, ProgIDs | Intcad's own `UpgradeCode`; `Program Files\Intcad`; `HKLM\Software\Intcad`; `Intcad.Design`, `Intcad.Model` | With upstream's `UpgradeCode` each product's installer removed the other. The `UpgradeCode` is now fixed for Intcad: never change it again. |
| deb / rpm package | `intcad` | Its own entry in the package database (`/var/lib/dpkg/info/intcad.list` is how the update check recognises a deb install). |
| Release files | `intcad-<version>-<platform>…`, and `Intcad-<version>-windows-x64.exe` | The update check and the download page pick files by these names. |
| Settings, recovery and cache folders | `Intcad` / `intcad` (section 4); nothing is taken from a `SolidCraft` folder | Two products no longer write one `graphics.json` and one recovery folder. |
| Programs | `intcad` and `intcad-cli` (`intcad.exe`, `intcad-cli.exe`, `Intcad.app/Contents/MacOS/Intcad`) | Both products' Linux packages installed `/usr/bin/solidcraft` and `/usr/bin/solidcraft-cli`, and both Windows installers put a `solidcraft-cli.exe` on `PATH`. |
| Folders the Linux packages install | `/usr/lib/intcad`, `/usr/share/doc/intcad` | The same paths in two packages. |
| deb / rpm relations | none to `solidcraft` or `solvecraft` | `Replaces`, `Conflicts` and `Provides: solvecraft` made installing Intcad remove upstream's earlier package. |
| MCP server entry | `intcad`, `intcad-live`; `serverInfo.name` is `intcad` (section 6) | `mcp setup` rewrote the entry upstream's setup writes. |
| Snap | `intcad`, with the commands `intcad` and `intcad.cli` | A snap name belongs to one product. |

Two things keep working for what was published under the earlier names:

- An AppImage looks in the latest release for its `.zsync` under the name it was published with.
  Each release carries the same `.zsync` under `solidcraft-…` and `solvecraft-…` as well
  (`packaging/linux/package.sh`), so the build before this change still finds its update.
- `packaging/web-channels.sh` takes each release's web zip under the name that release gave it,
  and checks it against that release's `SHA256SUMS.txt` under the same name, so stable can come
  from an old tag while the release candidate comes from a new one
  (`packaging/tests/web_channels_test.py`).

Intcad does not present itself as the successor of upstream's app: `io.tangencylabs.solidcraft`
is not added to the AppStream `<replaces>`, `X-Flatpak-RenamedFrom` or the Flatpak rebase list,
and settings kept in a Flatpak sandbox under that ID are not read. The older IDs listed below
are upstream's own history and are left as upstream wrote them.

Unchanged, and still shared with upstream by design: the design format (`.sc`, `solidcraft/N`,
`application/x-solidcraft`), `SOLIDCRAFT_*` variables, crate and Cargo package names
(`-p solidcraft`, `-p solidcraft-cli`) and the `solidcraft://` resource URIs.

## Package IDs

| Package | Decision | Why |
|---|---|---|
| Flatpak app ID / Linux desktop ID | changed in 0.8 from `ai.storyteller.solvecraft` to `ai.storyteller.solidcraft`, then with the move to Tangency Labs to `io.tangencylabs.solidcraft` | The project's own reverse-DNS domain. The `.desktop`, metainfo and MIME file names, the icon name, and the Wayland `app_id` / X11 class follow it. Existing installs are moved over; see [Linux app ID](#linux-app-id). |
| Flatpak remote file | `solidcraft.flatpakrepo`, plus a legacy `solvecraft.flatpakrepo` copy | Old links keep working; existing remotes point at the repo URL, which is unchanged. |
| macOS bundle ID | changed to `ai.storyteller.solidcraft`, then to `io.tangencylabs.solidcraft`; the CLI's signing identifier is `io.tangencylabs.solidcraft.cli` (was `ai.storyteller.solidcraft-cli`) | No updater depends on it; the app is drag-installed and replaces the old `SolidCraft.app`. Settings live in folders named after the product, not the bundle ID, so they stay. macOS ties privacy permissions (such as access to Documents or Downloads) to the bundle ID, so it asks for them once more after the update. |
| MSI `UpgradeCode` | **kept** (also through the `io.tangencylabs` move; the Windows build sets no AppUserModelID and its ProgIDs `Solidcraft.Design` / `Solidcraft.Model` don't carry the app ID, so nothing there changes) | So the new MSI upgrades the old install in place. Product name is SolidCraft; the default folder becomes `Program Files\SolidCraft` (the major upgrade removes the old folder). A custom install folder isn't remembered across this one upgrade. |
| deb / rpm package name | renamed to `solidcraft`; deb `Replaces`/`Conflicts`/`Provides: solvecraft`, rpm `Obsoletes`/`Provides: solvecraft` | Installing the new package replaces the old one instead of installing side by side. |
| AppImage | `solidcraft-<version>-linux-<arch>.AppImage`; the release also carries a copy of its `.zsync` under the old `solvecraft-…` name | AppImages already installed look for the old-named `.zsync`, so AppImageUpdate keeps finding updates. |

## Linux app ID

Upstream's releases used `ai.storyteller.solvecraft` up to 0.7, then `ai.storyteller.solidcraft`
until the move to Tangency Labs, then `io.tangencylabs.solidcraft`. Intcad's own Flatpak app id is
`ai.intgral.intcad` (the identity table above); it never had an earlier Flatpak app id of its
own. The settings and autosave folders are named after the product, not the app ID, so outside
the Flatpak sandbox they don't move. The remaining compatibility shims are:

| Shim | Where | What it does |
|---|---|---|
| AppStream `<replaces>` / `<provides>` | `assets/platforms/linux/ai.intgral.intcad.metainfo.xml.in` | Software centres treat the new id as the successor of `ai.storyteller.solidcraft` and `ai.storyteller.solvecraft` and of their `.desktop` ids. |
| `X-Flatpak-RenamedFrom` | `assets/platforms/linux/ai.intgral.intcad.desktop` (both old `.desktop` ids) | Desktops that honour it (GNOME Shell) keep favourites and pinned launchers pointing at the renamed entry. |
| The old window `app_id` as eframe's legacy storage key | `adopt_legacy_settings` (`LEGACY_APP_ID`) | Window state that releases before the settings rename kept under `~/.local/share/ai.storyteller.solvecraft` is still copied (section 4). |
| Old AppImage menu entries | `packaging/linux/AppRun` | An AppImage's first run removes the `ai.storyteller.solidcraft.desktop` and `ai.storyteller.solvecraft.desktop` entries and icons that an earlier AppImage installed in `~/.local/share` (only entries it wrote itself), so the menu doesn't show the app twice. Test: `packaging/tests/apprun_test.py`. |

There is no read-only grant and `solidcraft_io::legacy_flatpak_paths` is empty: settings and
autosaves kept under an upstream app id are another product's and are never read (section 4).
The old `app/ai.storyteller.*` refs are upstream's history; Intcad does not mark them end-of-life
or rebase them, so Flatpak does not move their `~/.var/app` data into Intcad. An install made next
to the old app starts with Intcad's own settings; the old folders are left unchanged.

deb and rpm need no shim: the package (`solidcraft`) is unchanged, and an upgrade removes the old
`/usr/share/applications/ai.storyteller.*.desktop`, metainfo, MIME and icon files
because the new version doesn't ship them; the post-install script refreshes the desktop, MIME
and icon caches, so no stale menu entry is left. The Snap is unaffected apart from its
`common-id`: snapd names its menu entry after the snap.

Flatpak installed from a downloaded `.flatpak` bundle (no update feed): install the new bundle,
launch it once (it starts with Intcad's own settings; data under an upstream app id is not
copied), then `flatpak uninstall --user ai.storyteller.solidcraft` (or
`ai.storyteller.solvecraft`).

**Removal:** upstream's old `app/ai.storyteller.*` refs are upstream's to keep or remove; Intcad
does not mark or rebase them. Intcad's own `app/ai.intgral.intcad` refs follow normal release
retention. `<replaces>`, `<provides>` and `X-Flatpak-RenamedFrom` are harmless and can stay.
Tests: `flatpak_twins_do_not_read_upstream_solidcraft_data`,
`first_launch_adopts_the_previous_release_settings`.

## macOS

Releases before 0.4 were installed as `SolveCraft.app`. To update, quit the app, drag
`SolidCraft.app` from the DMG onto Applications, move `SolveCraft.app` to the Trash, and
replace any Dock shortcut with the old name (open `SolidCraft.app`, then
**Options > Keep in Dock**). Designs and settings are kept (see 4).

## Flatpak remotes added from the former website address

The feed has moved twice: from the SolveCraft address, then from `bherbruck.github.io/solidcraft`
when the project moved to `docs.solidcraft.io` (#1920). A remote added from either address no
longer finds updates. Point it at the current one:
`flatpak remote-modify --user --url=https://docs.solidcraft.io/flatpak/repo/ solidcraft`
(use your remote's name: `solvecraft` for one added before 0.4), then `flatpak update --user`.

## The geometry kernel

The kernel is **solidcore**, vendored in `solidcore/` with its history, licence and attribution
files (Apache-2.0; a Rust port of SMLib and SM_API from USD BRep). The workspace takes
`solidcore`, `solidcore-record`, `solidcore-ops`, `solidcore-tess` and `solidcore-step` from that
tree by path; there is no git revision pin and no second copy of the kernel. `solidcore/` is
excluded from this workspace's lints and has its own toolchain file, as upstream publishes it.

`crates/kernel` is the only crate that speaks to the kernel's modelling layer; `crates/smlib` and
`crates/geom` keep the boundary types. A kernel change is made in `solidcore/` and carried here by
updating the vendored tree, never by forking a second kernel in `crates/`.

## Draft design containers

The #303 draft used `solvecraft-container/1`. `crates/io/src/design.rs` accepts that
marker alongside `solidcraft-container/1`, and writes only the new marker. Optional
old previews are discarded; required document/blob validation is unchanged. The
`draft_container_marker_remains_readable` regression covers this shim.

## Split pieces in designs saved before format 14

Builds before format 14 named the pieces a cut splits a body into in the old kernel's shell
order, and the design's later features name the piece they work on by that name (`Body1`,
`Body1 (1)`). SMLib orders pieces otherwise. Such a feature (`shell_piece_order` in the file)
names its pieces so its later features find them: each name goes to the piece the points those
features picked lie on. A name no later feature uses keeps SMLib's order. Designs saved since
format 14 name pieces after their faces and are not affected.
