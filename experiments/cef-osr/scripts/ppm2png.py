#!/usr/bin/env python3
"""Convert the probe's PPM (P6) dump to PNG without third-party packages."""
import struct
import sys
import zlib


def convert(source: str, target: str) -> None:
    with open(source, "rb") as handle:
        data = handle.read()
    if not data.startswith(b"P6\n"):
        raise SystemExit("not a P6 PPM")
    parts = data.split(b"\n", 3)
    width, height = (int(value) for value in parts[1].split())
    pixels = parts[3]
    stride = width * 3
    raw = bytearray()
    for row in range(height):
        raw.append(0)
        raw.extend(pixels[row * stride:(row + 1) * stride])

    def chunk(kind: bytes, payload: bytes) -> bytes:
        return (
            struct.pack(">I", len(payload))
            + kind
            + payload
            + struct.pack(">I", zlib.crc32(kind + payload) & 0xFFFFFFFF)
        )

    png = b"\x89PNG\r\n\x1a\n"
    png += chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0))
    png += chunk(b"IDAT", zlib.compress(bytes(raw), 9))
    png += chunk(b"IEND", b"")
    with open(target, "wb") as handle:
        handle.write(png)


if __name__ == "__main__":
    convert(sys.argv[1], sys.argv[2])
