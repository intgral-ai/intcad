# DFM check

Run: `run-1beae2b62b3c4809-f25a45fe2df23e14`
Project: `af242da7-f1f8-46ab-97eb-e567758e9a34`
Model: `sha256:c0353c20259fde66551c1dc6f304b89c15d3ce359cb871530edf8982a797ecf8`
Configuration digest: `2eda2417b83903b9dd84328e5bd3a1ec8415d7c333899d3d59a60a112d4de1e9`

Passed: 3 | Checked: 3 | Advisory: 0 | Failed: 0 | Unsupported: 9 | Not applicable: 0 | Not run: 0

Result: not a pass. At least one check has no passing, confirmed result.

## Findings

- `sheet-1/A1` `unsupported` (Body1), required 0.010 mm2 (unconfirmed), code `CANNOT_UNFOLD`
- `sheet-2/A1` `passed` (Body2), required 0.010 mm2 (unconfirmed)
- `sheet-2/F1` `checked` (Body2)
- `sheet-2/F10` `unsupported` (Body2), code `RULE_NOT_IMPLEMENTED`
- `sheet-2/F11` `unsupported` (Body2), code `NO_SHOP_NUMBER`
- `sheet-2/F12` `unsupported` (Body2), code `RULE_NOT_IMPLEMENTED`
- `sheet-2/F13` `unsupported` (Body2), code `RULE_NOT_IMPLEMENTED`
- `sheet-2/F2` `checked` (Body2), measured 1.200 mm
- `sheet-2/F3` `checked` (Body2)
- `sheet-2/F4` `unsupported` (Body2), code `COVERAGE_NO_REQUIREMENT`
- `sheet-2/F5` `passed` (Body2), measured 3.000 mm, required 2.000 mm (factory confirmed)
- `sheet-2/F6` `passed` (Body2), measured 5.000 mm, required 3.000 mm (factory confirmed)
- `sheet-2/F7` `unsupported` (Body2), code `CRITICAL_NOT_DECLARED`
- `sheet-2/F8` `unsupported` (Body2), code `RULE_NOT_IMPLEMENTED`
- `sheet-2/F9` `unsupported` (Body2), code `RULE_NOT_IMPLEMENTED`

## Not checked

- Tolerances other than declared hole-to-hole dimensions, bend sequence, collisions, grain and cut-outs that reach the outline were not checked.
- Sheet 1 (`Body1`): the sheet metal body was changed after its sheet-metal features by Cube5, so it cannot be unfolded and the hole rules were not run
- Sheet 2 (`Body2`): the App did not measure bend runs, relief ends, relief sizes, webs, countersinks.
- Sheet 2 (`Body2`): sm.tolerance_across_bends was not verified.
- Sheet 2 (`Body2`): sm.countersink_depth was not verified.
- Sheet 2 (`Body2`): sm.countersink_to_edge was not verified.
- Sheet 2 (`Body2`): sm.countersink_to_bend was not verified.
- Sheet 2 (`Body2`): sm.hole_spacing was not verified.
- Sheet 2 (`Body2`): sm.countersink_land was not verified.
- Sheet 2 (`Body2`): sm.countersink_angle was not verified.
- Sheet 1 (`Body1`): the flat self-overlap check could not measure this body.
