"""Prove a built binary embeds the reviewed public account tuple.

`apps/solidcraft/src/account.rs` reads the three values with `option_env!`, so they are
literal bytes in the executable. This check reads the compiled binary, requires each exact
value, and prints only the tuple's SHA-256 and the names of anything missing. It runs on the
build host after packaging and never launches the desktop app.
"""

import argparse
import os
import sys
from pathlib import Path

# `python -I` keeps the script's directory off sys.path; the shared validator is its sibling.
sys.path.insert(0, str(Path(__file__).resolve().parent))

from staging_account_config import KEYS, PRODUCTION_SNAPSHOT_ENV, SNAPSHOT_ENV, parse_snapshot, tuple_sha256


def embedded_values(binary: bytes, values: dict[str, str]) -> list[str]:
    missing = []
    for key in KEYS:
        if values[key].encode("utf-8") not in binary:
            missing.append(key)
    return missing


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path)
    parser.add_argument("--snapshot-env", default=None, choices=(SNAPSHOT_ENV, PRODUCTION_SNAPSHOT_ENV))
    parser.add_argument("--expect-tuple-sha256", default=None)
    args = parser.parse_args()

    import os

    env_name = args.snapshot_env or (
        PRODUCTION_SNAPSHOT_ENV if os.environ.get(PRODUCTION_SNAPSHOT_ENV) else SNAPSHOT_ENV
    )
    try:
        values = parse_snapshot(os.environ.get(env_name, ""), env_name)
    except ValueError as error:
        raise SystemExit(str(error)) from None

    if not args.binary.is_file():
        raise SystemExit(f"not a file: {args.binary}")
    data = args.binary.read_bytes()
    missing = embedded_values(data, values)
    if missing:
        raise SystemExit(f"{args.binary} does not embed the configured account: missing {', '.join(missing)}")

    digest = tuple_sha256(values)
    if args.expect_tuple_sha256 and args.expect_tuple_sha256 != digest:
        raise SystemExit(
            f"embedded account tuple {digest} does not match the expected {args.expect_tuple_sha256}"
        )
    print(f"embedded account tuple: {digest}; binary bytes: {len(data)}; all three values present")


if __name__ == "__main__":
    sys.exit(main())
