#!/usr/bin/env python3
"""Verify the Assistant page and its records inside an installed SolidCraft.app.

The page is not a folder of files in the app: solidcraft embeds packages/assistant-web/dist in
its executable with include_bytes!. This check therefore looks for each built file's exact bytes
inside Contents/MacOS/SolidCraft, and for the records the package carries beside it:

  Contents/Resources/Assistant/inventory.json
  Contents/Resources/Licenses/ASSISTANT-THIRD-PARTY-NOTICES.txt

It never reads the build tree as if it were the installed app: the caller must pass an .app copied
out of the disk image (packaging/macos/installed-check.sh does).

  python3 packaging/macos/verify-assistant-bundle.py --app path/to/SolidCraft.app
"""
import argparse
import json
import sys
from pathlib import Path

FAILURES = 0


def check(passed, what):
    global FAILURES
    print(("ok: " if passed else "FAIL: ") + what, flush=True)
    if not passed:
        FAILURES += 1
    return passed


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--app", required=True, help="the SolidCraft.app copied out of the disk image")
    parser.add_argument("--source", default=str(Path(__file__).resolve().parents[2] / "packages" / "assistant-web"),
                        help="the page's source folder in this checkout")
    args = parser.parse_args()

    app = Path(args.app).resolve()
    source = Path(args.source).resolve()
    exe = app / "Contents" / "MacOS" / "SolidCraft"
    inventory = app / "Contents" / "Resources" / "Assistant" / "inventory.json"
    notices = app / "Contents" / "Resources" / "Licenses" / "ASSISTANT-THIRD-PARTY-NOTICES.txt"

    if not check(app.is_dir(), f"the app is a bundle directory ({app})"):
        return 1
    if not check(exe.is_file(), f"the app has its executable ({exe})"):
        return 1

    binary = exe.read_bytes()
    dist = source / "dist"
    files = sorted(path for path in dist.iterdir() if path.is_file())
    if not check(bool(files), f"the built page has files ({dist})"):
        return 1
    for path in files:
        data = path.read_bytes()
        check(data in binary, f"the installed executable embeds {path.name} byte for byte ({len(data)} bytes)")

    source_inventory = source / "inventory.json"
    if check(inventory.is_file(), f"the app carries the page inventory ({inventory})"):
        check(inventory.read_bytes() == source_inventory.read_bytes(),
              "the installed inventory is the one this checkout built from")
        try:
            record = json.loads(inventory.read_text(encoding="utf-8"))
        except (OSError, ValueError) as error:
            record = None
            check(False, f"the installed inventory is valid JSON ({error})")
        if isinstance(record, dict):
            commit = (record.get("mastra") or {}).get("commit")
            check(isinstance(commit, str) and len(commit) == 40 and all(c in "0123456789abcdef" for c in commit),
                  "the installed inventory names the pinned Mastra commit")
            packages = record.get("packages")
            check(isinstance(packages, list) and len(packages) > 0,
                  "the installed inventory lists the page's third-party packages")
        else:
            check(False, "the installed inventory is a JSON object")
    else:
        check(False, f"the app carries the page inventory ({inventory})")

    source_notices = source / "THIRD-PARTY-NOTICES.txt"
    if check(notices.is_file(), f"the app carries the Assistant third-party notices ({notices})"):
        check(notices.read_bytes() == source_notices.read_bytes(),
              "the installed notices are the builder's notices, byte for byte")
    else:
        check(False, f"the app carries the Assistant third-party notices ({notices})")

    return 1 if FAILURES else 0


if __name__ == "__main__":
    sys.exit(main())
