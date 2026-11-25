"""Prove a built binary embeds the reviewed public account tuple.

Check for each exact value in the executable, separately in both supported Mach-O slices
for a universal macOS app. Print only the tuple digest, architecture and field names.
"""

import argparse
import mmap
import os
import struct
import sys
from pathlib import Path

# `python -I` keeps the script's directory off sys.path; the shared validator is its sibling.
sys.path.insert(0, str(Path(__file__).resolve().parent))

from staging_account_config import KEYS, PRODUCTION_SNAPSHOT_ENV, SNAPSHOT_ENV, parse_snapshot, tuple_sha256, verify_snapshot


def macho_slices(binary: mmap.mmap) -> dict[str, tuple[int, int]]:
    if len(binary) < 8:
        raise ValueError("truncated universal Mach-O header")
    magic, count = struct.unpack_from(">II", binary)
    if magic not in (0xCAFEBABE, 0xCAFEBABF) or count != 2:
        raise ValueError("expected a universal Mach-O with two slices")
    entry = struct.Struct(">IIIII" if magic == 0xCAFEBABE else ">IIQQII")
    header_end = 8 + count * entry.size
    if len(binary) < header_end:
        raise ValueError("truncated universal Mach-O directory")
    names = {0x01000007: "x86_64", 0x0100000C: "arm64"}
    slices = {}
    for i in range(count):
        cpu, _, start, size, *_ = entry.unpack_from(binary, 8 + i * entry.size)
        arch = names.get(cpu)
        if arch is None or arch in slices or start < header_end or size < 32 or start + size > len(binary):
            raise ValueError("invalid universal Mach-O directory")
        if struct.unpack_from("<II", binary, start) != (0xFEEDFACF, cpu):
            raise ValueError("Mach-O slice header disagrees with the directory")
        slices[arch] = start, start + size
    first, second = sorted(slices.values())
    if first[1] > second[0]:
        raise ValueError("overlapping universal Mach-O slices")
    return slices


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path)
    parser.add_argument("--macho-universal", action="store_true")
    parser.add_argument("--snapshot-env", default=None, choices=(SNAPSHOT_ENV, PRODUCTION_SNAPSHOT_ENV))
    parser.add_argument("--expect-tuple-sha256", default=None)
    args = parser.parse_args()

    env_name = args.snapshot_env or (
        PRODUCTION_SNAPSHOT_ENV if os.environ.get(PRODUCTION_SNAPSHOT_ENV) else SNAPSHOT_ENV
    )
    try:
        if args.macho_universal and sum(bool(os.environ.get(key)) for key in (SNAPSHOT_ENV, PRODUCTION_SNAPSHOT_ENV)) != 1:
            raise ValueError("universal Mach-O verification requires exactly one account snapshot")
        snapshot = os.environ.get(env_name, "")
        values = (verify_snapshot(snapshot, os.environ.get("GITHUB_SHA", ""))
                  if args.macho_universal and env_name == SNAPSHOT_ENV else parse_snapshot(snapshot, env_name))
        with args.binary.open("rb") as stream, mmap.mmap(stream.fileno(), 0, access=mmap.ACCESS_READ) as binary:
            slices = macho_slices(binary) if args.macho_universal else {"": (0, len(binary))}
            missing = [
                f"{arch}: {key}" if arch else key
                for arch, (start, end) in slices.items() for key in KEYS
                if binary.find(values[key].encode(), start, end) < 0
            ]
            size = len(binary)
    except (OSError, ValueError) as error:
        raise SystemExit(str(error)) from None

    if missing:
        raise SystemExit(f"{args.binary} does not embed the configured account: missing {', '.join(missing)}")

    digest = tuple_sha256(values)
    if args.expect_tuple_sha256 and args.expect_tuple_sha256 != digest:
        raise SystemExit(
            f"embedded account tuple {digest} does not match the expected {args.expect_tuple_sha256}"
        )
    architectures = " in " + ", ".join(sorted(slices)) if args.macho_universal else ""
    print(f"embedded account tuple: {digest}; binary bytes: {size}; all three values present{architectures}")


if __name__ == "__main__":
    sys.exit(main())
