# DFM check

Run: `run-c0e57b0d94f24d89-af40b576404f49bf`
Project: `af242da7-f1f8-46ab-97eb-e567758e9a34`
Model: `sha256:e7afa0bd11f8d0478ce638f67b16cab72baf4fbd7af46632a45750cc41584502`
Configuration digest: `1fc6ebd624710ba859401e0859e8e52e027499af545eacbc3a4520dd45c4c926`

Passed: 3 | Checked: 4 | Advisory: 0 | Failed: 0 | Unsupported: 4 | Not applicable: 0 | Not run: 0

Result: not a pass. At least one check has no passing, confirmed result.

## Findings

- `sheet-1/A1` `unsupported` (Body1), required 0.010 mm2 (unconfirmed), code `CANNOT_UNFOLD`
- `sheet-2/A1` `passed` (Body2), required 0.010 mm2 (unconfirmed)
- `sheet-2/F1` `checked` (Body2)
- `sheet-2/F2` `checked` (Body2), measured 1.200 mm
- `sheet-2/F3` `checked` (Body2)
- `sheet-2/F4` `unsupported` (Body2), code `COVERAGE_NO_REQUIREMENT`
- `sheet-2/F5` `passed` (Body2), measured 3.000 mm, required 2.000 mm (factory confirmed)
- `sheet-2/F6` `passed` (Body2), measured 5.000 mm, required 3.000 mm (factory confirmed)
- `sheet-2/F7` `unsupported` (Body2), code `CRITICAL_NOT_DECLARED`
- `sheet-2/F8` `checked` (Body2)
- `sheet-2/F9` `unsupported` (Body2), code `NO_SHOP_NUMBER`

## Not checked

- Tolerances other than declared hole-to-hole dimensions, bend sequence, collisions, grain and cut-outs that reach the outline were not checked.
- Sheet 1 (`Body1`): the sheet metal body was changed after its sheet-metal features by Cube5, so it cannot be unfolded and the hole rules were not run
- Sheet 2 (`Body2`): sm.tolerance_across_bends was not verified.
- Sheet 2 (`Body2`): sm.hole_spacing was not verified.
- Sheet 1 (`Body1`): the flat self-overlap check could not measure this body.
