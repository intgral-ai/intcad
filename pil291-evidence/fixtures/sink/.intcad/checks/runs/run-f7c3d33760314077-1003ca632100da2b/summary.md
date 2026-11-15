# DFM check

Run: `run-f7c3d33760314077-1003ca632100da2b`
Project: `5a66f35e-d067-408e-aa27-54fcb1da9a40`
Model: `sha256:8ce95ab5ac231c88dc9de7fba106918d8aa84c7f3812bed5cfb3c0e997261b71`
Configuration digest: `0d31260efd99f20055324cfa457ec54594de6072a05a59d8baec639dad3cc242`

Passed: 12 | Checked: 9 | Advisory: 0 | Failed: 2 | Unsupported: 1 | Not applicable: 0 | Not run: 0

Result: check failed: 2 finding(s) break a confirmed requirement.

## Findings

- `sheet-1/A1` `passed` (Body1), required 0.010 mm2 (unconfirmed)
- `sheet-1/F1` `checked` (Body1)
- `sheet-1/F10` `passed` (Body1), required 0.000 mm (factory confirmed)
- `sheet-1/F11` `passed` (Body1), measured 28.300 mm, required 3.000 mm (factory confirmed)
- `sheet-1/F12` `checked` (Body1)
- `sheet-1/F13` `checked` (Body1)
- `sheet-1/F14` `unsupported` (Body1), code `CRITICAL_NOT_DECLARED`
- `sheet-1/F15` `failed` (Body1), measured 2.050 mm, required 0.750 mm (factory confirmed), code `COUNTERSINK_DEPTH_ABOVE_LIMIT`
- `sheet-1/F16` `passed` (Body1), measured 26.250 mm, required 3.000 mm (factory confirmed)
- `sheet-1/F17` `passed` (Body1), measured 20.750 mm, required 3.000 mm (factory confirmed)
- `sheet-1/F18` `checked` (Body1)
- `sheet-1/F19` `passed` (Body1), measured 1.000 mm, required 0.000 mm (factory confirmed)
- `sheet-1/F2` `checked` (Body1), measured 2.500 mm
- `sheet-1/F20` `checked` (Body1)
- `sheet-1/F21` `checked` (Body1)
- `sheet-1/F22` `failed` (Body1), measured 0.450 mm, required 0.500 mm (factory confirmed), code `COUNTERSINK_LAND_BELOW_LIMIT`
- `sheet-1/F23` `passed` (Body1), measured 90.000 deg, required 1.000 deg (factory confirmed)
- `sheet-1/F3` `checked` (Body1), measured 1.000 mm
- `sheet-1/F4` `checked` (Body1)
- `sheet-1/F5` `passed` (Body1), measured 3.000 mm, required 0.500 mm (factory confirmed)
- `sheet-1/F6` `passed` (Body1), measured 14.500 mm, required 8.000 mm (factory confirmed)
- `sheet-1/F7` `passed` (Body1), measured 3.400 mm, required 3.000 mm (factory confirmed)
- `sheet-1/F8` `passed` (Body1), measured 100.000 mm, required 500.000 mm (factory confirmed)
- `sheet-1/F9` `passed` (Body1), measured 22.800 mm, required 2.500 mm (factory confirmed)

## Not checked

- Tolerances other than declared hole-to-hole dimensions, bend sequence, collisions, grain and cut-outs that reach the outline were not checked.
- Sheet 1 (`Body1`): sm.tolerance_across_bends was not verified.
