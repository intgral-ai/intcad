# DFM check

Run: `run-44418324135c40bc-2c9139833c74fcec`
Project: `7256e4ca-e070-4030-a4d9-e4f6557becc3`
Model: `sha256:7255d7623f8173f8e7457dbcd39b0c9a80a9262126698746e7e9058956eeb073`
Configuration digest: `6cc287e6b4624dca6735401305320d03a6c97c52d212df2572ab27093ca382ad`

Passed: 10 | Checked: 7 | Advisory: 0 | Failed: 6 | Unsupported: 1 | Not applicable: 0 | Not run: 0

Result: check failed: 6 finding(s) break a confirmed requirement.

## Findings

- `sheet-1/A1` `passed` (Body1), required 0.010 mm2 (unconfirmed)
- `sheet-1/F1` `checked` (Body1)
- `sheet-1/F10` `failed` (Body1), required 0.000 mm (factory confirmed), code `RELIEF_NO_MARGIN`
- `sheet-1/F11` `failed` (Body1), required 0.000 mm (factory confirmed), code `RELIEF_NO_MARGIN`
- `sheet-1/F12` `failed` (Body1), required 0.000 mm (factory confirmed), code `RELIEF_NO_MARGIN`
- `sheet-1/F13` `failed` (Body1), required 0.000 mm (factory confirmed), code `RELIEF_NO_MARGIN`
- `sheet-1/F14` `passed` (Body1), measured 28.500 mm, required 3.000 mm (factory confirmed)
- `sheet-1/F15` `passed` (Body1), measured 92.000 mm, required 5.000 mm (factory confirmed)
- `sheet-1/F16` `passed` (Body1), measured 11.000 mm, required 18.400 mm (factory confirmed)
- `sheet-1/F17` `passed` (Body1), measured 0.100 mm, required 0.050 mm (factory confirmed)
- `sheet-1/F18` `checked` (Body1)
- `sheet-1/F19` `passed` (Body1), measured 8.662 mm, required 4.000 mm (factory confirmed)
- `sheet-1/F2` `checked` (Body1), measured 2.000 mm
- `sheet-1/F20` `failed` (Body1), measured 1.000 mm, required 4.000 mm (factory confirmed), code `HOLE_SPACING_BELOW_LIMIT`
- `sheet-1/F21` `failed` (Body1), measured 2.000 mm, required 0.000 mm (factory confirmed), code `BEND_RADII_MIXED`
- `sheet-1/F22` `checked` (Body1)
- `sheet-1/F23` `checked` (Body1)
- `sheet-1/F3` `checked` (Body1), measured 3.000 mm
- `sheet-1/F4` `checked` (Body1)
- `sheet-1/F5` `passed` (Body1), measured 1.000 mm, required 0.500 mm (factory confirmed)
- `sheet-1/F6` `unsupported` (Body1), code `FLANGE_CROSSES_CUT`
- `sheet-1/F7` `passed` (Body1), measured 3.000 mm, required 3.000 mm (factory confirmed)
- `sheet-1/F8` `passed` (Body1), measured 92.000 mm, required 500.000 mm (factory confirmed)
- `sheet-1/F9` `passed` (Body1), measured 3.500 mm, required 2.500 mm (factory confirmed)

## Not checked

- Tolerances other than declared hole-to-hole dimensions, bend sequence, collisions, grain and cut-outs that reach the outline were not checked.
- Sheet 1 (`Body1`): sm.min_flange was not verified.
