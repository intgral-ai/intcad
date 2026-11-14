#!/usr/bin/env python3
"""Rename the displayed product name from open-skyfall to Intcad in a checkout of the desktop app.

Only the name people see changes: interface text and translations, window and installer names,
and the wording of the MCP server, the command line and the docs. Internal names stay
(`solidcraft` crates, binaries, folders, environment variables, format tags, app ids, the MCP
server's own name), and so does everything this script protects below.

It does two things: every `open-skyfall` becomes `Intcad`, and the phrases listed in `VISIBLE`,
which still said SolidCraft, say Intcad. Intcad is the product; the company that maintains it is
Intgral, and text that already names the company (authors, the manufacturer and vendor fields of
the packages, "maintained by Intgral") is not touched.

Run from the repository root: `python3 scripts/display_name_to_intcad.py [--check]`. With
`--check` it changes nothing and exits 1 if a file still needs the rename. It is safe to run
twice: the second run changes nothing. Run it again after a merge that brings `open-skyfall` or a
listed phrase back. It does not convert other text that says SolidCraft: new wording from
upstream, new translations included, needs a look and, where it is shown, a new entry in `VISIBLE`.
"""
import pathlib, re, subprocess, sys

# Kept verbatim wherever they occur.
PROTECTED = [
    r"open-skyfall-desktop",                    # the GitHub repository, until its owner renames it
    r"open-skyfall-mark[A-Za-z0-9.-]*",         # logo file names
    r"open-skyfall-brand\.md",                  # an attribution file's name
    r"user_agent\(\"open-skyfall[a-z-]*\"\)",   # what the app calls itself to web servers (updater)
]
# Files left alone as a whole.
SKIP_FILES = {"NOTICE", "Cargo.lock", "THIRD-PARTY-LICENSES.txt", "ATTRIBUTION.md", "LICENSE-MIT", "LICENSE-APACHE"}
SKIP_DIRS = ("vendor/",)
SELF = "scripts/display_name_to_intcad.py"     # this file names what it replaces
# Forms that are not stem plus the same ending: (file, old form, new form). None is needed for
# Intcad; the inflected forms in the translations (Finnish, Hungarian, Czech, Turkish and others)
# keep their endings.
SPECIAL = []

# Wording that still said SolidCraft where a person or a calling program reads it: the macOS
# application menu, the dialogs shown when the app cannot start, the messages of refused files
# and commands, what the Assistant's agents are told, the help text of commands, and two lines of
# the installers (the disk image's read-me, the Flatpak repository page). Each entry
# is an exact phrase in one file; inside it SolidCraft becomes Intcad ("a SolidCraft" becomes
# "an Intcad"). Tests and the two guide pages that quote a message are listed with it.
# Everything else that says SolidCraft is a name something depends on (folders, file names,
# identifiers, what exported files carry, the updater) and stays.
VISIBLE = {
    "apps/solidcraft/src/main.rs": ['"SolidCraft could not open its window', '"SolidCraft could not start"'],
    "apps/solidcraft/src/native_menu.rs": ['Submenu::new("SolidCraft"', 'name: Some("SolidCraft"', '"Hide SolidCraft"', '"{} SolidCraft"'],
    "apps/solidcraft-cli/tests/run_results.rs": ["update SolidCraft"],
    "assets/locales/pending/mastra-backend.json": ["Reinstall SolidCraft from", "SolidCraft を再インストール", "重新安装 SolidCraft"],
    "assets/platforms/macos/README.txt": ["and SolidCraft copies"],
    "crates/doc/src/drawing.rs": ["not a SolidCraft drawing"],
    "crates/doc/src/drawing_tests.rs": ["not a SolidCraft ", "newer SolidCraft", "update SolidCraft"],
    "crates/doc/src/format.rs": ["not a SolidCraft ", "newer SolidCraft", "update SolidCraft"],
    "crates/doc/src/format_offsets.rs": ['contains("newer SolidCraft")'],
    "crates/doc/src/metadata.rs": ["SolidCraft's are "],
    "crates/engine/src/agent_setup.rs": ["not a plain SolidCraft entry"],
    "crates/engine/src/cmd/bom.rs": ["(SolidCraft's keys, as"],
    "crates/engine/src/cmd/bom_tests.rs": ["SolidCraft's are "],
    "crates/engine/src/cmd/component.rs": ["open in another SolidCraft;"],
    "crates/engine/src/cmd/file.rs": ['"a SolidCraft drawing"', "when another SolidCraft has the design open"],
    "crates/engine/src/cmd/frame.rs": ["each SolidCraft's own parametric definition"],
    "crates/engine/src/cmd/metadata.rs": ["each of SolidCraft's design", "SolidCraft's keys, the ones", "SolidCraft's are "],
    "crates/engine/src/cmd/sketch_import.rs": ["no entities SolidCraft reads"],
    "crates/engine/src/control_auth.rs": ["or restart SolidCraft", "if SolidCraft was force-quit"],
    "crates/engine/src/schema/component.rs": ["open in another SolidCraft."],
    "crates/engine/src/schema/metadata.rs": ["and SolidCraft's own design", "Each of SolidCraft's design properties", "one of SolidCraft's keys", "text for SolidCraft's keys"],
    "crates/io/src/design.rs": ["is a SolidCraft drawing, not a design"],
    "crates/io/src/design/drawing.rs": ["is a SolidCraft design, not a drawing"],
    "crates/io/src/design/drawing_tests.rs": ["is a SolidCraft drawing, not a design", "is a SolidCraft design, not a drawing"],
    "crates/io/src/lib.rs": ["which SolidCraft can't read", "is a SolidCraft drawing, not a design"],
    "crates/io/src/limits.rs": ["or start SolidCraft with --max-file-size"],
    "crates/io/src/ply.rs": ["longer than SolidCraft reads"],
    "crates/io/src/point_cloud.rs": ["no points SolidCraft can read"],
    "crates/io/src/svg.rs": ["largest number SolidCraft can store", "SolidCraft reads plain SVG only", "which SolidCraft does not follow", "no shapes SolidCraft reads"],
    "crates/ui-egui/src/agent_chat/acp.rs": ["inside SolidCraft, a parametric"],
    "crates/ui-egui/src/agent_chat/api_client.rs": ["no SolidCraft tools in this session", "the SolidCraft tools stopped", "the SolidCraft tools did not answer"],
    "crates/ui-egui/src/agent_chat/host_ai/task.rs": ["SolidCraft task sessions serve no host tools"],
    "crates/ui-egui/src/agent_chat/host_bridge.rs": ["the SolidCraft chat bridge"],
    "crates/ui-egui/src/agent_chat/packaged_runtime.rs": ["reinstall SolidCraft", "this SolidCraft "],
    "crates/ui-egui/src/cli_link.rs": ["move SolidCraft to /Applications."],
    "crates/ui-egui/src/help.rs": ["next to SolidCraft;"],
    "crates/ui-egui/src/keymap.rs": ["is used by SolidCraft itself", '"SolidCraft shortcuts.json"'],
    "crates/ui-egui/src/settings_profiles.rs": ["not a SolidCraft settings profile"],
    "crates/ui-egui/src/toolbar_custom.rs": ["not a SolidCraft shortcuts file"],
    "docs/book/src/guide/troubleshooting.md": ["saved by a newer SolidCraft"],
    "docs/control-protocol.md": ["or restart SolidCraft."],
    "packaging/flatpak-site.sh": ["SolidCraft Flatpak repository", "Add the SolidCraft repository"],
}


def visible(check):
    """Apply `VISIBLE`; the number of files that changed (or would)."""
    changed = 0
    for name, phrases in VISIBLE.items():
        path = pathlib.Path(name)
        if not path.is_file():
            print(f"missing: {name}", file=sys.stderr)
            continue
        text = path.read_text(encoding="utf-8")
        new = text
        for phrase in phrases:
            renamed = re.sub(r"\ba SolidCraft", "an Intcad", phrase).replace("SolidCraft", "Intcad")
            new = new.replace(phrase, renamed)
        if new != text:
            changed += 1
            if not check:
                path.write_text(new, encoding="utf-8")
    return changed

def main():
    check = "--check" in sys.argv
    files = subprocess.run(["git", "grep", "-ilI", "open-skyfall"], capture_output=True, text=True).stdout.split("\n")
    changed, kept = 0, 0
    for name in filter(None, files):
        if pathlib.Path(name).name in SKIP_FILES or name.startswith(SKIP_DIRS) or name == SELF:
            continue
        path = pathlib.Path(name)
        text = path.read_text(encoding="utf-8")
        held = []
        def hold(match):
            held.append(match.group(0))
            return f"\x00{len(held) - 1}\x00"
        new = text
        for pattern in PROTECTED:
            new = re.sub(pattern, hold, new)
        for file, old, replacement in SPECIAL:
            if name == file:
                new = new.replace(old, replacement)
        new = re.sub(r"[Oo]pen-skyfall", "Intcad", new)
        new = re.sub(r"\x00(\d+)\x00", lambda m: held[int(m.group(1))], new)
        kept += len(held)
        if new != text:
            changed += 1
            if not check:
                path.write_text(new, encoding="utf-8")
    changed += visible(check)
    print(f"{'would change' if check else 'changed'} {changed} files; kept {kept} protected names")
    return 1 if check and changed else 0

if __name__ == "__main__":
    sys.exit(main())
