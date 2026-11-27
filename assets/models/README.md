# Classic 3DBenchy preview

`test_boat.bin` is the mesh embedded in the **3D Mouse** preferences preview. It
is not a CAD template or design file. The app does not fetch or reduce meshes at
runtime; normal builds read the checked-in binary, not the authoring tools.

The only source mesh is CreativeTools' [classic single-part 3DBenchy STL][source],
revision `b542785f9642ab39db8a778a0ccc934b3ebd829a`. [3DBenchy.com][licence]
identifies the classic model as CC0 1.0 Universal and credits Daniel Norée for
the design and Paulo Kiefe for naming and publishing it. This derivation
does not establish the provenance of earlier packed exports.

## Regenerate offline

Download the publisher files below into a scratch input directory with the
given names. Do not check in those downloads or the intermediate mesh. The
recipe checks each input digest before running the simplifier; filenames alone
are not a match.

| Name | Pinned source | SHA-256 |
| --- | --- | --- |
| `classic.stl` | [CreativeTools STL][source] | `6ab57f1c3f8e86bc3cbd302c6fa6270acf06277c6335454e922419c25d42e97e` |
| `meshoptimizer.h` | [meshoptimizer v0.25 header][header] | `c896a6d31dae787202172098e560eeef7780519e5ca085cf1a03e3f07ef0bb8a` |
| `simplifier.cpp` | [meshoptimizer v0.25 source][simplifier] | `a92aca2fa4a0e8e1aca91dbdd23b1c92a1ec2ff748a0827ce47cc8fb940477e8` |
| `LICENSE.md` | [meshoptimizer v0.25 MIT license][mit] | `e4a26033e3551fb2722888949fbb41e77aee628e8e8f04dcffeee301aa7e5634` |

Run from the repository root on x86-64 Linux. The reference tools are GCC
16.2.0, Python 3.14.7, NumPy 2.5.4 and GNU binutils 2.46 (`ld`, `readelf`,
and `nm`). Put those executables on `PATH`. Activate a Python environment
with that NumPy version, or set `PYTHONPATH` to the installed NumPy
`site-packages` directory before running the block. These are authoring
tools only. The temporary simplifier links no native runtime libraries and
is neither checked in nor shipped.

```sh
(
    set -eu
    # Set INPUTS to the directory containing the exact publisher files above.
    python3 -B -c 'import sys, numpy; print(sys.version.split()[0], numpy.__version__)'
    c++ -dumpfullversion
    ld --version | sed -n '1p'
    mkdir -p target
    out=$(mktemp -d "$PWD/target/benchy.XXXXXXXX")
    python3 -B assets/models/regenerate_test_boat.py "$INPUTS" "$out/first"
    python3 -B assets/models/regenerate_test_boat.py "$INPUTS" "$out/second"
    cmp "$out/first/reduced.stl" "$out/second/reduced.stl"
    cmp "$out/first/test_boat.bin" "$out/second/test_boat.bin"
    cmp "$out/first/test_boat.bin" assets/models/test_boat.bin
)
```

The recipe welds at 0.001 mm and removes triangles with twice-area at or below
`1e-9 mm²`. Meshoptimizer at `6daea4695c48338363b08022d2fb15deaef6ac09`
reduces to 3,900 indices with `meshopt_SimplifyRegularize` only and relative
error limit `0.02`. It keeps the axes and scale, saves triangles in returned
order as little-endian binary STL with zero attributes and computed normals,
and passes that file through `make_test_boat.py` with the existing 40° crease
split and quantization.

| Output | Expected SHA-256 |
| --- | --- |
| `reduced.stl` | `60e309e59e81e81d72881adea789b25a4e25ed6e9722925c566f78cf4752e9df` |
| `test_boat.bin` | `d22cb6c68d56eb4e55f15fd8edc23a7f3ef21856ea7d288f069cf14fb68de0ef` |

The result has 1,300 triangles, 1,171 shaded vertices and 18,363 bytes. It is
Z-up and about 59.764 × 30.866 × 48.000 mm. After welding the deliberate
normal splits by position, the packed mesh is one closed, consistently
wound component without degenerate faces or self-intersections. For a
visual check, open the 3D Mouse page in the desktop Preferences.

[source]: https://github.com/CreativeTools/3DBenchy/blob/b542785f9642ab39db8a778a0ccc934b3ebd829a/Single-part/3DBenchy.stl
[licence]: https://www.3dbenchy.com/license/
[header]: https://github.com/zeux/meshoptimizer/blob/6daea4695c48338363b08022d2fb15deaef6ac09/src/meshoptimizer.h
[simplifier]: https://github.com/zeux/meshoptimizer/blob/6daea4695c48338363b08022d2fb15deaef6ac09/src/simplifier.cpp
[mit]: https://github.com/zeux/meshoptimizer/blob/6daea4695c48338363b08022d2fb15deaef6ac09/LICENSE.md
