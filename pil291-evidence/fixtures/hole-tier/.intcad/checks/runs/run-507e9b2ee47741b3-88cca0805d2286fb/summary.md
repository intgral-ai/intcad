# DFM check

Run: `run-507e9b2ee47741b3-88cca0805d2286fb`
Project: `af242da7-f1f8-46ab-97eb-e567758e9a34`
Model: `sha256:2cecd6e2b8b6fb9609d09715288278f3a8038499e7805d07d22acf24feb1fa57`
Configuration digest: `2eda2417b83903b9dd84328e5bd3a1ec8415d7c333899d3d59a60a112d4de1e9`

Passed: 1 | Checked: 3 | Advisory: 0 | Failed: 20 | Unsupported: 8 | Not applicable: 0 | Not run: 0

Result: check failed: 20 finding(s) break a confirmed requirement.

## Findings

- `sheet-1/F1` `checked` (Body1)
- `sheet-1/F2` `checked` (Body1), measured 2.000 mm
- `sheet-1/F3` `checked` (Body1)
- `sheet-1/F4` `unsupported` (Body1), code `COVERAGE_NO_REQUIREMENT`
- `sheet-1/F5` `failed` (Body1), measured 3.000 mm, required 3.500 mm (factory confirmed), code `HOLE_DIAMETER_BELOW_LIMIT`
- `sheet-1/F6` `failed` (Body1), measured 3.000 mm, required 3.500 mm (factory confirmed), code `HOLE_DIAMETER_BELOW_LIMIT`
- `sheet-1/F7` `failed` (Body1), measured 3.000 mm, required 3.500 mm (factory confirmed), code `HOLE_DIAMETER_BELOW_LIMIT`
- `sheet-1/F8` `failed` (Body1), measured 3.000 mm, required 3.500 mm (factory confirmed), code `HOLE_DIAMETER_BELOW_LIMIT`
- `sheet-1/F9` `failed` (Body1), measured 3.000 mm, required 3.500 mm (factory confirmed), code `HOLE_DIAMETER_BELOW_LIMIT`
- `sheet-1/F10` `failed` (Body1), measured 3.000 mm, required 3.500 mm (factory confirmed), code `HOLE_DIAMETER_BELOW_LIMIT`
- `sheet-1/F11` `failed` (Body1), measured 3.000 mm, required 3.500 mm (factory confirmed), code `HOLE_DIAMETER_BELOW_LIMIT`
- `sheet-1/F12` `failed` (Body1), measured 3.000 mm, required 3.500 mm (factory confirmed), code `HOLE_DIAMETER_BELOW_LIMIT`
- `sheet-1/F13` `failed` (Body1), measured 3.000 mm, required 3.500 mm (factory confirmed), code `HOLE_DIAMETER_BELOW_LIMIT`
- `sheet-1/F14` `failed` (Body1), measured 3.000 mm, required 3.500 mm (factory confirmed), code `HOLE_DIAMETER_BELOW_LIMIT`
- `sheet-1/F15` `failed` (Body1), measured 3.000 mm, required 3.500 mm (factory confirmed), code `HOLE_DIAMETER_BELOW_LIMIT`
- `sheet-1/F16` `failed` (Body1), measured 3.000 mm, required 3.500 mm (factory confirmed), code `HOLE_DIAMETER_BELOW_LIMIT`
- `sheet-1/F17` `failed` (Body1), measured 3.000 mm, required 3.500 mm (factory confirmed), code `HOLE_DIAMETER_BELOW_LIMIT`
- `sheet-1/F18` `failed` (Body1), measured 3.000 mm, required 3.500 mm (factory confirmed), code `HOLE_DIAMETER_BELOW_LIMIT`
- `sheet-1/F19` `failed` (Body1), measured 3.000 mm, required 3.500 mm (factory confirmed), code `HOLE_DIAMETER_BELOW_LIMIT`
- `sheet-1/F20` `failed` (Body1), measured 3.000 mm, required 3.500 mm (factory confirmed), code `HOLE_DIAMETER_BELOW_LIMIT`
- `sheet-1/F21` `failed` (Body1), measured 3.000 mm, required 3.500 mm (factory confirmed), code `HOLE_DIAMETER_BELOW_LIMIT`
- `sheet-1/F22` `failed` (Body1), measured 3.000 mm, required 3.500 mm (factory confirmed), code `HOLE_DIAMETER_BELOW_LIMIT`
- `sheet-1/F23` `failed` (Body1), measured 3.000 mm, required 3.500 mm (factory confirmed), code `HOLE_DIAMETER_BELOW_LIMIT`
- `sheet-1/F24` `failed` (Body1), measured 3.000 mm, required 3.500 mm (factory confirmed), code `HOLE_DIAMETER_BELOW_LIMIT`
- `sheet-1/F25` `passed` (Body1), measured 5.000 mm, required 5.000 mm (factory confirmed)
- `sheet-1/F26` `unsupported` (Body1), code `CRITICAL_NOT_DECLARED`
- `sheet-1/F27` `unsupported` (Body1), code `RULE_NOT_IMPLEMENTED`
- `sheet-1/F28` `unsupported` (Body1), code `RULE_NOT_IMPLEMENTED`
- `sheet-1/F29` `unsupported` (Body1), code `RULE_NOT_IMPLEMENTED`
- `sheet-1/F30` `unsupported` (Body1), code `NO_SHOP_NUMBER`
- `sheet-1/F31` `unsupported` (Body1), code `RULE_NOT_IMPLEMENTED`
- `sheet-1/F32` `unsupported` (Body1), code `RULE_NOT_IMPLEMENTED`

## Not checked

- Tolerances other than declared hole-to-hole dimensions, bend sequence, collisions, grain and cut-outs that reach the outline were not checked.
- Sheet 1 (`Body1`): the App did not measure bend runs, relief ends, relief sizes, webs, countersinks.
- Sheet 1 (`Body1`): sm.tolerance_across_bends was not verified.
- Sheet 1 (`Body1`): sm.countersink_depth was not verified.
- Sheet 1 (`Body1`): sm.countersink_to_edge was not verified.
- Sheet 1 (`Body1`): sm.countersink_to_bend was not verified.
- Sheet 1 (`Body1`): sm.hole_spacing was not verified.
- Sheet 1 (`Body1`): sm.countersink_land was not verified.
- Sheet 1 (`Body1`): sm.countersink_angle was not verified.
