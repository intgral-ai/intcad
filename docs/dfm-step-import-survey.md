# STEP import and sheet-metal conversion survey (PIL-297)

Date: 2026-11-13 (re-run against current code). Scope: measure real sheet-metal STEP files with INTdfm/OCCT, import them into
SolidCraft, convert them to native sheet metal where possible, export and re-measure. A separate
native export -> import round trip checks the exporter/importer without conversion.

**The downloaded files are not in this repository.** Every manifest entry says the licence is
unclear (`see repo`). They live only in a local evidence directory outside the repositories and must not be
committed or shipped.

Baseline: SolidCraft develop `1a45733ef96b53990fec8bd7794563eebc6facdb`; INTdfm
`22c3f3a` (contract baseline `2f24830fcd84c8e3047eb6643139b94c27b8c1b6`); OCCT `cadquery-ocp-novtk 8.0.1.1`.

## Samples and hashes

All eight real files were fetched from `INTdfm/tests/data/manifest.json`; byte counts and SHA-256
were re-verified in the re-run. Licence note is `unclear` for every one, so they stay outside
this repository. The five synthetic parts are built locally by SolidCraft and are counted
separately.

| File | SHA-256 | Type |
|---|---|---|
| `openCOVI__Sheet_Metal_Base.STEP` | `6e306ece43a7a5501b90b1802314974cb43e363c9fbac1c8de1acc16361350ef` | flat 480×220×0.531 mm |
| `abdrumm__Motor_Mount_Sheetmetal.step` | `b2b255d04ac5af8d317cf340ee0172b466d50f0f16bf0a836721e0ba89ecc963` | flat 62.5×54.6×3.175 mm, 9 holes |
| `abdrumm__Fan_Shroud_sheetmetal.step` | `3d82e6534992cef2ad6ed87010645cd64ecdf7d88d0fcd248d9127c4113f1e11` | 149.75° arch |
| `openCOVI__Sheet_metal_longer_side.STEP` | `8e1a28fc765aab097516ab32f268f5c0318bc3b92aacaf4ec67fc682e1857205` | 1×90° r3, 10 holes |
| `polpo__picogus_2.0_sheet_metal_bracket.step` | `7e21dd3693abb531664ac89e24e916dce67873a3e07c8eac646da36ddd460183` | 6 bends, mixed radii, 5 holes |
| `zachmohr__Base_SheetMetal.step` | `c10646512db4f8905bac96219b70cdc38ef7ea033a9e9918c25c4125e416cc5a` | 2×20°, inch file |
| `jaimeuyd__ServoBracketBent.step` | `a48e75d5a6524468d52f9748f406c70efa9fd11e490d32e07d86360b42c516ae` | B-spline bracket |
| `jaimeuyd__MotorBracketBent.step` | `4c811664812f8a02eedc9630037d43bcd27ba0dd5078faece1ea3750b522004e` | B-spline bracket, import refused |

Reproduction: the local evidence directory (not committed) holds the re-run driver, per-part
configs and logs. It uses the prebuilt `solidcraft-cli` at develop `1a45733` and INTdfm `22c3f3a`.
The scripts download only the public files above, verify their hashes, and keep all output outside
both repositories.

## Method

1. Fetch each file from `INTdfm/tests/data/manifest.json`; verify byte count and SHA-256.
2. Measure the original with INTdfm's reader (`engine.run`): input gate, solids/shells/faces,
   volume, size, paired thickness, holes (diameter and folded position), bends (angle, radius,
   length).
3. `file.insert_step` into a fresh SolidCraft design; keep the body name and any warnings.
4. `sheet.convert` with a rule whose thickness is the measured paired thickness and whose bend
   radius is the measured inside radius. Picks come from `model.faces`/`model.edges`: a point on
   a sharp edge, or a tangent edge between a cylinder and a plane for a radiused bend.
5. Export the flat pattern to STEP, re-read with the same INTdfm reader, and pair holes/bends by
   nearest position (pairing tolerance 0.05 mm).

SolidCraft (SMLib) and INTdfm (OCCT) count faces differently. Compare face counts only within one
tool (before vs after), never across tools.

## Results

| Part | Original | Import | Convert | Original vs converted | Affects the first three hole rules? |
|---|---|---|---|---|---|
| `openCOVI__Sheet_Metal_Base` — flat 0.531 mm | gate pass, 1 solid, 0 holes, 0 bends | rc 0, nothing skipped | rc 0, no picks | volume 0.000, thickness 0.000, bbox 0 | no holes or bends |
| `abdrumm__Motor_Mount` — flat 3.175 mm, 9 holes | gate pass, 9 holes, 0 bends | rc 0 | rc 0, no picks | volume 0.000000; 9/9 holes, max diameter Δ 0.0000 mm, max position Δ 0.00000 mm | no; diameters/positions preserved |
| `openCOVI__Sheet_metal_longer_side` — 1×90° r3, 10 holes | gate pass, 1 bend, 10 holes | rc 0, **1 warning** (4 edges off surfaces ≤ 2.255e-3 mm) | rc 0, 1 tangent-edge pick | volume +1.767 mm³ (+0.0034 %); bend 90°/r3/545 exact; holes 10/10 exact | no; radius and holes exact |
| `zachmohr__Base_SheetMetal` — 2×20°, inch file | gate pass, 2 bends r1.6002, 0 holes | rc 0 | rc 0, 2 picks | volume −0.0000; both bends angle/radius/length Δ 0 | no holes |
| `polpo__picogus_2.0` — 6 bends, mixed r 0/0.9906, 5 holes | gate pass, 6 bends, 5 holes, 4 spline faces | rc 0 | **rc 1** with all or the first 4 picks; **one-bend run rc 0 with no warning but is not this part** | partial run: −8.372 % volume, 5 of 6 bends lost, 2 ⌀3.175 holes lost; surviving holes' nearest-bend gaps inflate to 103.64/36.72/19.82 mm | **yes**; false-clear risk |
| `abdrumm__Fan_Shroud` — 149.75° arch, 1 bend, 8 holes | gate pass, 1 bend r19.05 | rc 0 | **rc 1** (68 pick/fixed-face runs) | not measured | **yes**; no converted part |
| `jaimeuyd__ServoBracketBent` — B-spline faces | gate pass, 0 bends recognised, 12 other openings | rc 0 | **rc 1** (16 runs) | not measured | **yes**; no converted part |
| `jaimeuyd__MotorBracketBent` | gate pass | **rc 1, 0 bodies** (`knot multiplicity` on a pcurve) | not reached | not measured | unknown; import refused |

Import outcomes: **7/8** one body; 1 refused (`MotorBracketBent`, knot multiplicity).
Conversion outcomes: **4 accepted** (3 exact; `longer_side` +0.003438 % volume with every bend
and hole exact), **2 refused** (`Fan_Shroud`, `Servo`), **1 partial and rejected** (`polpo`),
**1 not attempted** (import failed). A small sample, not a product success rate.

Every accepted conversion was checked bend-by-bend and hole-by-hole: each bend radius/angle/
length/position and each hole diameter/folded position matched within 0.05 mm pairing and the
measurement tolerance. The detailed per-hole and per-bend tables are kept in the local evidence directory.

**All five converted exports are still assemblies.** `intdfm check` refused 5/5 with
`INPUT_ASSEMBLY` (2 products, 1 assembly link, 1 solid definition), even though INTdfm's reader
sees one closed solid in each. Fidelity above is reader-level only; no converted part reached a
full `intdfm check`.

## Per-bend and per-hole detail (accepted conversions)

All comparisons use the same INTdfm reader on the original and the converted export; holes and
bends are paired by nearest folded position within 0.05 mm.

| Part | Bends (original -> converted) | Holes (original -> converted) |
|---|---|---|
| openCOVI base | none | none |
| Motor Mount | none | 9 -> 9; every diameter Δ 0.0000 mm, every folded position Δ 0.000000 mm |
| longer_side | 1 -> 1: 90° / r3.0 / L545, position Δ 0 | 10 -> 10; every ⌀4.2 Δ 0.0000 mm, every position Δ 0.000000 mm |
| zachmohr | 2 -> 2: both 20° / r1.6002 / L254, positions Δ 0 | none |
| polpo (partial, rejected) | 6 -> 1: only 90° / r0.9906 / L18.796 survives; 5 lost | 5 -> 3; 2 × ⌀3.175 lost; surviving diameters/positions exact; nearest-bend gaps 5.5499/7.3326/6.5205 -> 103.6447/36.7244/19.8244 mm |

## Native export -> import round trip (separate check)


Five native sheet-metal parts (flat plate, 2-flange tray with 5 holes, 4-flange box, 45°+90° part
with 3 holes, channel with 2 holes) were exported to STEP and re-imported. All five were exact:
volume Δ 0.000 mm³, identical face counts, bend angle/radius/length and hole diameters identical,
thickness 1.5 mm exactly.

This proves SolidCraft's own export/import path for native sheet metal. It does **not** prove
external STEP -> native conversion.

## Blockers and interpretation

1. **Import can reject a solid OCCT considers valid** (`SMLib export: pcurve … knot multiplicity`).
2. **B-spline/HOOPS exports have no usable planar-face sheet walk.** The converter refuses
   "curved faces in a converted sheet".
3. **Bends over 90°** can land the walk on the other cylindrical skin of the same arch.
4. **Mixed radii and short flanges** fail the one-rule-radius conversion or the setback/gap fit.
5. **Picks are mandatory** and only discoverable from `model.edges`; there is no auto-detection.
6. **Every STEP exported from a design containing an import remains an assembly**, so INTdfm
   refuses it as `INPUT_ASSEMBLY`. Converted parts could only be measured with INTdfm's reader,
   not `intdfm check`.
7. **A partial conversion can be silent and is the most dangerous failure for DFM**: `polpo`
   accepted 1 of 6 picks with rc 0 and no warning, losing 5 bends and 2 holes. A lost bend can
   make `sm.hole_to_bend` not applicable; a surviving but wrong bend inflates the gap and gives a
   false clear. `sheet.convert` should reject an incomplete pick set, or require an explicit
   partial-conversion acknowledgement. This is a proposal; no converter change was applied.
8. **Imported B-spline geometry can already differ before conversion**: `Servo` imported
   0.0448 % larger than INTdfm read the original. Conversion fidelity cannot be better than the
   import; this is a separate pre-conversion difference.

## What is still unverified

- A converted part read back through `intdfm check` (blocked by the assembly wrapper; 0/5).
- An external 4-flange tray/box and a sharp-bend part that converts cleanly.
- A >90° or hem external part that must be accepted or clearly refused.
- A second non-mm file and a formed-feature file that must be refused cleanly.
- Whether any of these public files may be used beyond local evaluation. Licence notes are
  unclear for all of them.

## Minimum acceptance set for the next round

1. One external flat part with holes (already covered by Motor Mount).
2. One external 90° part with holes (already covered by longer_side).
3. One external box/tray with four flanges.
4. One external >90° or hem part.
5. One external part that must be refused (open shell, assembly, mesh).
6. A negative conversion test: a multi-bend part with only some bends picked must fail, not
   return rc 0 with a partial body.
7. One native part exported -> imported -> checked through the full `dfm.sheet` chain.
