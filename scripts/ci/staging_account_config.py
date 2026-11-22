"""Snapshot and validate the public build inputs for staging desktop packages."""

import argparse
import ipaddress
import json
import os
import re
from collections.abc import Mapping
from pathlib import Path

CLIENT_ID = "INTCAD_WORKOS_PUBLIC_CLIENT_ID"
AUTHKIT_ORIGIN = "INTCAD_AUTHKIT_VERIFICATION_ORIGIN"
PRODUCT_ORIGIN = "INTCAD_PRODUCT_HTTPS_ORIGIN"
KEYS = (CLIENT_ID, AUTHKIT_ORIGIN, PRODUCT_ORIGIN)
SNAPSHOT_ENV = "STAGING_ACCOUNT_CONFIG"

CLIENT_PATTERN = re.compile(r"client_[A-Za-z0-9]+")
ORIGIN_PATTERN = re.compile(r"https://(\[[0-9a-fA-F:.]+\]|[a-zA-Z0-9.-]+)(?::([0-9]{1,5}))?")
HOST_LABEL = re.compile(r"[A-Za-z0-9](?:[A-Za-z0-9-]{0,61}[A-Za-z0-9])?")


def _is_https_origin(value: str) -> bool:
    match = ORIGIN_PATTERN.fullmatch(value)
    if not match:
        return False
    host, port = match.groups()
    if port is not None and not 1 <= int(port) <= 65535:
        return False
    if host.startswith("["):
        try:
            ipaddress.IPv6Address(host[1:-1])
        except ValueError:
            return False
        return True
    host = host.removesuffix(".")
    return len(host) <= 253 and all(HOST_LABEL.fullmatch(label) for label in host.split("."))


def validate_config(values: Mapping[str, str]) -> dict[str, str]:
    invalid = []
    result = {}
    for key in KEYS:
        value = values.get(key)
        if not isinstance(value, str) or not (
            CLIENT_PATTERN.fullmatch(value) if key == CLIENT_ID else _is_https_origin(value)
        ):
            invalid.append(key)
        else:
            result[key] = value
    if invalid:
        raise ValueError("missing or invalid staging keys: " + ", ".join(invalid))
    if set(values) != set(KEYS):
        # Do not echo arbitrary input (including unrecognized key names).
        raise ValueError("unexpected staging config keys")
    return result


def parse_snapshot(snapshot: str) -> dict[str, str]:
    def unique_keys(pairs):
        values = dict(pairs)
        if len(values) != len(pairs):
            raise ValueError
        return values

    try:
        values = json.loads(snapshot, object_pairs_hook=unique_keys)
    except (ValueError, RecursionError):
        raise ValueError(f"invalid JSON in {SNAPSHOT_ENV}") from None
    if not isinstance(values, dict):
        raise ValueError(f"{SNAPSHOT_ENV} must contain a JSON object")
    return validate_config(values)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=("snapshot", "inject"))
    args = parser.parse_args()
    try:
        if args.command == "snapshot":
            values = validate_config({key: os.environ.get(key, "") for key in KEYS})
            destination = "GITHUB_OUTPUT"
            text = "config=" + json.dumps(values, separators=(",", ":")) + "\n"
        else:
            values = parse_snapshot(os.environ.get(SNAPSHOT_ENV, ""))
            destination = "GITHUB_ENV"
            text = "".join(f"{key}={value}\n" for key, value in values.items())
    except ValueError as error:
        raise SystemExit(str(error)) from None
    with Path(os.environ[destination]).open("a", encoding="utf-8", newline="\n") as stream:
        stream.write(text)


if __name__ == "__main__":
    main()
