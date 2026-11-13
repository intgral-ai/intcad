# STEP import and sheet-metal conversion survey (PIL-297)

Date: 2026-11-12. Scope: measure real sheet-metal STEP files with INTdfm/OCCT, import them into
SolidCraft, convert them to native sheet metal where possible, export and re-measure. A separate
native export -> import round trip checks the exporter/importer without conversion.

**The downloaded files are not in this repository.** Every manifest entry says the licence is
unclear (`see repo`). They live only in `/tmp/opencode/step-survey/parts` and must not be
committed or shipped.

Baseline: SolidCraft develop `1a45733ef96b53990fec8bd7794563eebc6facdb`; INTdfm
`2f24830fcd84c8e3047eb6643139b94c27b8c1b6`; OCCT `cadquery-ocp-novtk 8.0.1.1`.

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
| `openCOVI__Sheet_metal_longer_side` — 1×90° r3, 10 holes | gate pass, 1 bend, 10 holes | rc 0 | rc 0, 1 tangent-edge pick | volume +1.767 mm³ (+0.0034 %); bend 90°/r3/545 exact; holes 10/10 exact | no; radius and holes exact |
| `zachmohr__Base_SheetMetal` — 2×20°, inch file | gate pass, 2 bends r1.6002, 0 holes | rc 0 | rc 0, 2 picks | volume −0.0000; both bends angle/radius/length Δ 0 | no holes |
| `polpo__picogus_2.0` — 6 bends, mixed r 0/0.9906, 5 holes | gate pass, 6 bends, 5 holes, 4 spline faces | rc 0 | **rc 1** with all or the first 4 picks; one-bend run rc 0 but is not this part | partial run: −8.372 % volume, 5 of 6 bends lost, 2 holes lost | **yes**; a lost bend makes `sm.hole_to_bend` not applicable |
| `abdrumm__Fan_Shroud` — 149.75° arch, 1 bend, 8 holes | gate pass, 1 bend r19.05 | rc 0 | **rc 1** (56 pick/fixed-face runs) | not measured | **yes**; no converted part |
| `jaimeuyd__ServoBracketBent` — B-spline faces | gate pass, 0 bends recognised, 12 other openings | rc 0 | **rc 1** (16 runs) | not measured | **yes**; no converted part |
| `jaimeuyd__MotorBracketBent` | gate pass | **rc 1, 0 bodies** (`knot multiplicity` on a pcurve) | not reached | not measured | unknown; import refused |

Conversion outcomes: 3 full (one with a 0.0034 % volume difference), 1 partial rejected, 3
blocked, 1 import failure.

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
7. **A lost bend is the dangerous failure for DFM**: the converted part has fewer bends, so
   `sm.hole_to_bend` may become not applicable and look like a pass. A conversion must record the
   bend count before and after and refuse a mismatch.

## What is still unverified

- A converted part read back through `intdfm check` (blocked by the assembly wrapper).
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
6. One native part exported -> imported -> checked through the full `dfm.sheet` chain.
