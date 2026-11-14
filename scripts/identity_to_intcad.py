#!/usr/bin/env python3
"""Give a checkout of the desktop app Intcad's own identity on a user's machine.

`scripts/display_name_to_intcad.py` changed the name people read. This script changes what the
operating system and the installers go by, so an Intcad install is not an install of upstream
SolidCraft under another label:

  - the app id (macOS bundle id and document type, Linux desktop / AppStream / MIME / icon names,
    Flatpak and Arch ids): io.tangencylabs.solidcraft -> ai.intgral.intcad, file names included
  - the macOS bundle: SolidCraft.app -> Intcad.app
  - installer and release file names: solidcraft-<version>-... -> intcad-<version>-...
  - Windows: install folder, registry key, file-association ProgIds, and an UpgradeCode and
    component GUIDs of Intcad's own
  - the deb / rpm package name
  - the settings, data and cache folders, where docs, comments and messages spell them (the
    code that picks the folders and copies an earlier one on first launch is edited by hand)

Binary names (`solidcraft`, `solidcraft-cli`) and the MCP server name are a separate change. Until
it lands the two products still collide where those names are installed: the Linux packages
ship the same files under /usr, and on Windows both register `solidcraft.exe` and put their
folder on PATH. Only on macOS are they fully apart after this script.

It leaves alone, on purpose: crate names, `SOLIDCRAFT_*` variables, the design format
(`solidcraft/N`, `.sc`, `.solidcraft`, `application/x-solidcraft`), `solidcraft://` URIs, links
to upstream, licence and attribution text, and the older ids and names that
`docs/compatibility.md` keeps as shims for upstream's own history.

Run from the repository root: `python3 scripts/identity_to_intcad.py [--check]`. With `--check`
it changes nothing and exits 1 if a file still needs the change. It is safe to run twice. Run it
again after a merge from upstream, then read what it reports: an entry of `EXACT` that finds
neither its old nor its new text means upstream rewrote that line and the entry needs a look.
Follow a run with `cargo fmt --all`: the shorter name lets some lines join.
"""
import pathlib
import re
import subprocess
import sys

OLD_ID, NEW_ID = "io.tangencylabs.solidcraft", "ai.intgral.intcad"

# Files left alone as a whole.
SKIP_FILES = {
    "Cargo.lock", "THIRD-PARTY-LICENSES.txt", "NOTICE", "LICENSE-MIT", "LICENSE-APACHE",
    "scripts/identity_to_intcad.py",      # this file names what it replaces
    "scripts/display_name_to_intcad.py",  # describes the earlier state
    ".github/release-notes.md",           # upstream's notes for upstream's releases
    "docs/compatibility.md",              # the history of the ids; its Intcad section is written by hand
    "docs/beta-evidence.md",              # dated records of candidates that were built under the old names
    "docs/fork-governance.md",            # a dated inventory of what the repository held
}
SKIP_DIRS = ("vendor/",)

# A line that points at upstream's own repository or site keeps upstream's file names.
UPSTREAM_LINE = re.compile(r"(tangency-labs|bherbruck)/solidcraft|solidcraft\.io")

# Test data a signature was made over: the list of checksums the update tests verify against a
# fixed test key. Changing a byte of it makes the signature wrong, and there is no key to re-sign.
SIGNED_LINE = re.compile(r"const SIGNED_SUMS: &str")

# Applied to every other tracked text file, in order. A rule marked "line" is applied line by
# line and skips lines that match UPSTREAM_LINE or SIGNED_LINE.
RULES = [
    # The app id, wherever it is spelled.
    (re.compile(re.escape(OLD_ID)), NEW_ID),
    # The macOS bundle and the archive it is notarized in.
    (re.compile(r"SolidCraft\.app\b"), "Intcad.app"),
    (re.compile(r"SolidCraft-app\.zip"), "Intcad-app.zip"),
    # Installer and release file names: the name followed by a version or a platform. Not crate
    # names (solidcraft-engine), temporary folders (solidcraft-{name}) or the skills folders, and
    # not on a line that links to upstream's own downloads (see UPSTREAM_LINE).
    (re.compile(r"(?<![\w.])(?<!skills\\)solidcraft-(cli-)?(?=(\$|\{version|<version|\*-|[0-9]+\.[0-9]|linux-|windows-|macos-|freebsd-))"),
     lambda m: "intcad-" + (m.group(1) or ""), "line"),
    (re.compile(r"(?<![\w.])SolidCraft-(?=(\$|\{version|<version|[0-9]+\.[0-9]))"), "Intcad-", "line"),
    # The same names with a wildcard or an ellipsis for the version, and the browser build's zip.
    (re.compile(r"(?<![\w.])solidcraft-(?=(\*\.|…\.))"), "intcad-", "line"),
    (re.compile(r"(?<![\w.])solidcraft-web-(?=(\$|<version|\*|[0-9]+\.[0-9]))"), "intcad-web-", "line"),
    # Windows: the install folder, the registry key that remembers it, the file associations.
    (re.compile(r"([Pp]rogram [Ff]iles(?: \(x86\))?[\\/]{1,2})SolidCraft"), r"\1Intcad"),
    (re.compile(r"(program files[\\/]{1,2})solidcraft(?![\w.-])"), r"\1intcad"),
    (re.compile(r"(Software\\{1,2})SolidCraft\b"), r"\1Intcad"),
    (re.compile(r"\bSolidcraft\.(Design|Model)\b"), r"Intcad.\1"),
    # The deb package's file list, which tells the update check how the app was installed.
    (re.compile(r"dpkg/info/solidcraft\.list"), "dpkg/info/intcad.list"),
    # The per-user settings, data and cache folders, where docs, comments and messages spell them.
    # The code that picks the folders, and copies an earlier folder on first launch, is edited by
    # hand: apps/solidcraft/src/main.rs, crates/engine/src/{recovery,slicer_files}.rs,
    # crates/ui-egui/src/home.rs.
    (re.compile(r"(Application Support/|Library/Caches/)SolidCraft\b"), r"\1Intcad"),
    (re.compile(r"(%(?:LOCAL)?APPDATA%\\{1,2})SolidCraft\b"), r"\1Intcad"),
    (re.compile(r"((?:\.local/share|\.cache|XDG_(?:DATA|CACHE)_HOME)/)solidcraft\b"), r"\1intcad"),
    (re.compile(r"(AppData[\\/]Local[\\/])SolidCraft\b"), r"\1Intcad"),
]

# Exact text in one file: (old, new). Each old text must be there, or its new text already.
EXACT = {
    "assets/platforms/windows/solidcraft.wxs": [
        ("  <!-- Never change the UpgradeCode: it is what lets a new MSI replace every earlier install,\n"
         "       including those from before the SolveCraft -> SolidCraft rename (docs/compatibility.md). -->\n",
         "  <!-- Never change the UpgradeCode: it is what lets a new MSI replace every earlier Intcad\n"
         "       install. It is Intcad's own, not upstream SolidCraft's, so neither product's installer\n"
         "       replaces the other (docs/compatibility.md). -->\n"),
        ('UpgradeCode="6A145FD6-D285-428D-AE84-A69570CDF4DB"', 'UpgradeCode="984E28EB-86EB-46BE-908D-738A48FE6432"'),
        ('<Directory Id="INSTALLFOLDER" Name="SolidCraft" />', '<Directory Id="INSTALLFOLDER" Name="Intcad" />'),
        ('<Component Id="SolidcraftApp" Guid="A379D509-D776-4B34-A3A4-71FBF5724971">',
         '<Component Id="SolidcraftApp" Guid="52EE9723-50D7-463A-8F4B-4551B5D33517">'),
        ('<Component Id="SolidcraftLicences" Guid="CDEDEE8D-4BE9-4872-A2EA-EE623A6C7E91">',
         '<Component Id="SolidcraftLicences" Guid="2BC0E79E-2792-4849-B0BC-B8AB82D403EA">'),
    ],
    "packaging/linux/nfpm.yaml": [("\nname: solidcraft\n", "\nname: intcad\n")],
    # Where the installed-files check looks for the MSI's install.
    "packaging/windows/installed-check.ps1": [("$Dir = Join-Path $ProgramFiles 'SolidCraft'", "$Dir = Join-Path $ProgramFiles 'Intcad'")],
    # Folder names that are spelled apart from the path in front of them.
    "crates/gpu/src/concurrent.rs": [('ends_with(r"SolidCraft\\dxcompiler.dll")', 'ends_with(r"Intcad\\dxcompiler.dll")')],
    "examples/demo/enclosure_mcp.py": [('Path(os.environ["LOCALAPPDATA"]) / "SolidCraft" / "control"', 'Path(os.environ["LOCALAPPDATA"]) / "Intcad" / "control"')],
    "scripts/blur-evidence.ps1": [('Join-Path $env:LOCALAPPDATA "SolidCraft/control/$Port.token"', 'Join-Path $env:LOCALAPPDATA "Intcad/control/$Port.token"')],
    "docs/book/src/guide/troubleshooting.md": [("thumbnails in `$XDG_CACHE_HOME/SolidCraft` or `/tmp/SolidCraft`", "thumbnails in `$XDG_CACHE_HOME/Intcad` or `/tmp/Intcad`")],
    # The guide's table of how the update check recognises each kind of install.
    "docs/book/src/guide/updates.md": [("dpkg knows `solidcraft`", "dpkg knows `intcad`")],
    # The AppImage's opt-out marker sits in the app's data folder.
    "packaging/linux/AppRun": [('[ -e "$data/solidcraft/no-desktop-integration" ]', '[ -e "$data/intcad/no-desktop-integration" ]')],
    # The disk image's volume name (what Finder shows when it is mounted).
    "packaging/macos/package.sh": [('  SolidCraft "$DMG"\n', '  Intcad "$DMG"\n')],
}


def git(*args):
    return subprocess.run(["git", *args], capture_output=True, check=True).stdout.decode()


def text_files():
    for rel in git("ls-files", "-z").split("\0"):
        if not rel or rel in SKIP_FILES or rel.startswith(SKIP_DIRS):
            continue
        path = pathlib.Path(rel)
        if not path.is_file() or path.is_symlink():
            continue
        try:
            data = path.read_bytes()
        except OSError:
            continue
        if b"\0" in data[:8192]:
            continue
        try:
            yield rel, data.decode("utf-8")
        except UnicodeDecodeError:
            continue


def main():
    check = "--check" in sys.argv[1:]
    pending, problems = [], []
    for rel, old in text_files():
        new = old
        for pattern, replacement, *scope in RULES:
            if scope:
                new = "".join(
                    line if UPSTREAM_LINE.search(line) or SIGNED_LINE.search(line) else pattern.sub(replacement, line)
                    for line in new.splitlines(keepends=True)
                )
            else:
                new = pattern.sub(replacement, new)
        for before, after in EXACT.get(rel, []):
            if before in new:
                new = new.replace(before, after)
            elif after not in new:
                problems.append(f"{rel}: neither the old nor the new text of an EXACT entry is there: {before.strip()[:70]!r}")
        if new != old:
            pending.append(rel)
            if not check:
                pathlib.Path(rel).write_text(new, encoding="utf-8")
    renames = [rel for rel in git("ls-files", "-z").split("\0") if OLD_ID in rel]
    for rel in renames:
        if not check:
            target = rel.replace(OLD_ID, NEW_ID)
            pathlib.Path(target).parent.mkdir(parents=True, exist_ok=True)
            subprocess.run(["git", "mv", rel, target], check=True)
    for rel in EXACT:
        if not pathlib.Path(rel).is_file() and not pathlib.Path(rel.replace(OLD_ID, NEW_ID)).is_file():
            problems.append(f"{rel}: the file of an EXACT entry is gone")
    for line in problems:
        print(f"needs a look: {line}", file=sys.stderr)
    verb = "would change" if check else "changed"
    print(f"{verb} {len(pending)} files, {'would rename' if check else 'renamed'} {len(renames)}")
    if check and (pending or renames):
        for rel in pending + renames:
            print(f"  {rel}")
    return 1 if problems or (check and (pending or renames)) else 0


if __name__ == "__main__":
    sys.exit(main())
