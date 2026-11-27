#!/usr/bin/env python3
"""Offline authoring recipe for the embedded Classic 3DBenchy preview."""

import argparse
import ctypes as ct
from hashlib import sha256
import os
from pathlib import Path
import platform
import struct
import subprocess
import sys
from tempfile import TemporaryDirectory

import numpy as np

HERE = Path(__file__).resolve().parent
PINS = {
    "classic.stl": "6ab57f1c3f8e86bc3cbd302c6fa6270acf06277c6335454e922419c25d42e97e",
    "meshoptimizer.h": "c896a6d31dae787202172098e560eeef7780519e5ca085cf1a03e3f07ef0bb8a",
    "simplifier.cpp": "a92aca2fa4a0e8e1aca91dbdd23b1c92a1ec2ff748a0827ce47cc8fb940477e8",
    "LICENSE.md": "e4a26033e3551fb2722888949fbb41e77aee628e8e8f04dcffeee301aa7e5634",
}
OUTPUTS = {
    "reduced.stl": "60e309e59e81e81d72881adea789b25a4e25ed6e9722925c566f78cf4752e9df",
    "test_boat.bin": "d22cb6c68d56eb4e55f15fd8edc23a7f3ef21856ea7d288f069cf14fb68de0ef",
}
STL = np.dtype([("normal", "<f4", 3), ("vertices", "<f4", (3, 3)), ("attribute", "<u2")])
TARGET = 1300


def checked_bytes(path, expected):
    data = path.read_bytes()
    actual = sha256(data).hexdigest()
    if actual != expected:
        raise ValueError(f"{path.name}: SHA-256 mismatch (expected {expected}, got {actual})")
    return data


def compact(points, faces):
    used, indices = np.unique(faces, return_inverse=True)
    return points[used], indices.reshape(-1, 3)


def normals(points, faces):
    a, b, c = points[faces].astype(np.float64).transpose(1, 0, 2)
    return np.cross(b - a, c - a)


def compile_simplifier(work):
    shared = work / "simplifier.so"
    env = {k: v for k, v in os.environ.items() if k != "NIX_LDFLAGS"}
    env["LC_ALL"] = "C"
    subprocess.run([
        "c++", "-std=c++11", "-O2", "-fno-fast-math", "-ffp-contract=off", "-DNDEBUG",
        "-fPIC", "-fno-stack-protector", "-fno-exceptions", "-fno-rtti", "-fno-builtin",
        "-nostdlib", "-nodefaultlibs", "-shared",
        f"-Wl,-z,defs,--version-script={HERE / 'test_boat_simplifier.map'}",
        work / "simplifier.cpp", HERE / "test_boat_simplifier.cpp", "-o", shared,
    ], check=True, env=env)
    dynamic = subprocess.check_output(["readelf", "-d", shared], text=True, env=env)
    undefined = subprocess.check_output(["nm", "-Du", shared], text=True, env=env)
    if undefined.strip() or any(f"({key})" in dynamic for key in ("NEEDED", "RPATH", "RUNPATH")):
        raise ValueError("mesh simplifier has unexpected dynamic dependencies or search paths")
    return shared


def reduce_mesh(data, shared, destination):
    count, = struct.unpack_from("<I", data, 80)
    if len(data) != 84 + count * STL.itemsize:
        raise ValueError("bad STL size")
    tris = np.frombuffer(data, dtype=STL, offset=84)["vertices"]
    if not np.isfinite(tris).all():
        raise ValueError("non-finite vertex in STL")
    coordinates = np.round(tris.reshape(-1, 3).astype(np.float64), 3)
    coordinates[coordinates == 0] = 0  # canonicalize signed zero
    points, indices = np.unique(coordinates, axis=0, return_inverse=True)
    faces = indices.reshape(-1, 3)
    points, faces = compact(points, faces[np.linalg.norm(normals(points, faces), axis=1) > 1e-9])

    lib = ct.CDLL(str(shared))
    arena = ct.create_string_buffer(64 * 1024 * 1024)
    lib.test_boat_arena.argtypes = [ct.c_void_p, ct.c_size_t]
    lib.test_boat_arena.restype = None
    lib.test_boat_arena(arena, len(arena))
    simplify = lib.meshopt_simplify
    uints = np.ctypeslib.ndpointer(np.uint32, flags="C_CONTIGUOUS")
    floats = np.ctypeslib.ndpointer(np.float32, flags="C_CONTIGUOUS")
    simplify.argtypes = [
        uints, uints, ct.c_size_t, floats, ct.c_size_t, ct.c_size_t, ct.c_size_t,
        ct.c_float, ct.c_uint, ct.POINTER(ct.c_float),
    ]
    simplify.restype = ct.c_size_t
    faces = np.ascontiguousarray(faces, np.uint32)
    points = np.ascontiguousarray(points, np.float32)
    output = np.empty_like(faces)
    error = ct.c_float()
    # Only meshopt_SimplifyRegularize, at a relative error limit of 0.02.
    size = simplify(output, faces, faces.size, points, len(points), 12, 3 * TARGET, 0.02, 16, ct.byref(error))
    if size != 3 * TARGET or np.any(output.ravel()[:size] >= len(points)):
        raise ValueError(f"unexpected simplifier output ({size} indices)")
    points, faces = compact(points, output[:size // 3])
    stl = np.zeros(len(faces), dtype=STL)
    stl["vertices"] = points[faces]
    cross = normals(points, faces)
    stl["normal"] = cross / np.linalg.norm(cross, axis=1)[:, None]
    header = struct.pack("<80sI", b"CreativeTools Classic 3DBenchy - Intcad preview", len(faces))
    destination.write_bytes(header + stl.tobytes())


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("inputs", type=Path, help="directory with the four pinned files in README.md")
    parser.add_argument("output", type=Path, help="new directory for the two verified exports")
    args = parser.parse_args()
    try:
        if platform.system() != "Linux" or platform.machine() != "x86_64":
            raise ValueError("the authoring recipe is supported on x86-64 Linux")
        if args.output.exists() or args.output.is_symlink():
            raise ValueError(f"output already exists: {args.output}")
        stl, *rest = (checked_bytes(args.inputs / name, pin) for name, pin in PINS.items())
        with TemporaryDirectory(prefix="test-boat-") as tmp:
            work = Path(tmp)
            for name, data in zip(list(PINS)[1:], rest, strict=True):
                (work / name).write_bytes(data)
            reduced = work / "reduced.stl"
            reduce_mesh(stl, compile_simplifier(work), reduced)
            subprocess.run([sys.executable, "-B", HERE / "make_test_boat.py", reduced,
                            work / "test_boat.bin"], check=True)
            outputs = {name: checked_bytes(work / name, pin) for name, pin in OUTPUTS.items()}
        args.output.mkdir()
        for name, data in outputs.items():
            (args.output / name).write_bytes(data)
            print(f"{OUTPUTS[name]}  {args.output / name}")
    except (OSError, ValueError, subprocess.SubprocessError) as exc:
        parser.exit(1, f"{exc}\n")


if __name__ == "__main__":
    main()
