# DFM check

Run: `run-746f66113dbb4f11-d635b2bd3aac7266`
Project: `7256e4ca-e070-4030-a4d9-e4f6557becc3`
Model: `sha256:1a0ffb72d032c157d7f027913fa02e0811f9d97d395d2663cd477a16fc8ff22c`
Configuration digest: `e4d9acbadf04cf4b3a8fce981eea790afc224ef68a3269380bf24497d791193e`

Passed: 7 | Checked: 4 | Advisory: 0 | Failed: 2 | Unsupported: 12 | Not applicable: 0 | Not run: 0

Result: check failed: 2 finding(s) break a confirmed requirement.

## Findings

- `sheet-1/A1` `passed` (Body1), required 0.010 mm2 (unconfirmed)
- `sheet-1/F1` `checked` (Body1)
- `sheet-1/F10` `unsupported` (Body1), code `RULE_NOT_IMPLEMENTED`
- `sheet-1/F11` `passed` (Body1), measured 28.500 mm, required 3.000 mm (factory confirmed)
- `sheet-1/F12` `unsupported` (Body1), code `RULE_NOT_IMPLEMENTED`
- `sheet-1/F13` `unsupported` (Body1), code `RULE_NOT_IMPLEMENTED`
- `sheet-1/F14` `unsupported` (Body1), code `CRITICAL_NOT_DECLARED`
- `sheet-1/F15` `unsupported` (Body1), code `RULE_NOT_IMPLEMENTED`
- `sheet-1/F16` `unsupported` (Body1), code `RULE_NOT_IMPLEMENTED`
- `sheet-1/F17` `unsupported` (Body1), code `RULE_NOT_IMPLEMENTED`
- `sheet-1/F18` `passed` (Body1), measured 8.662 mm, required 4.000 mm (factory confirmed)
- `sheet-1/F19` `failed` (Body1), measured 1.000 mm, required 4.000 mm (factory confirmed), code `HOLE_SPACING_BELOW_LIMIT`
- `sheet-1/F2` `checked` (Body1), measured 2.000 mm
- `sheet-1/F20` `failed` (Body1), measured 2.000 mm, required 0.000 mm (factory confirmed), code `BEND_RADII_MIXED`
- `sheet-1/F21` `unsupported` (Body1), code `RULE_NOT_IMPLEMENTED`
- `sheet-1/F22` `unsupported` (Body1), code `RULE_NOT_IMPLEMENTED`
- `sheet-1/F23` `unsupported` (Body1), code `RULE_NOT_IMPLEMENTED`
- `sheet-1/F24` `unsupported` (Body1), code `RULE_NOT_IMPLEMENTED`
- `sheet-1/F3` `checked` (Body1), measured 3.000 mm
- `sheet-1/F4` `checked` (Body1)
- `sheet-1/F5` `passed` (Body1), measured 2.000 mm, required 0.500 mm (factory confirmed)
- `sheet-1/F6` `unsupported` (Body1), code `RULE_NOT_IMPLEMENTED`
- `sheet-1/F7` `passed` (Body1), measured 3.000 mm, required 3.000 mm (factory confirmed)
- `sheet-1/F8` `passed` (Body1), measured 91.000 mm, required 500.000 mm (factory confirmed)
- `sheet-1/F9` `passed` (Body1), measured 2.500 mm, required 2.500 mm (factory confirmed)

## Not checked

- Tolerances other than declared hole-to-hole dimensions, bend sequence, collisions, grain and cut-outs that reach the outline were not checked.
- Sheet 1 (`Body1`): the App did not measure bend runs, relief ends, relief sizes, webs, countersinks.
- Sheet 1 (`Body1`): sm.min_flange was not verified.
- Sheet 1 (`Body1`): sm.bend_relief_present was not verified.
- Sheet 1 (`Body1`): sm.min_web_parallel_bends was not verified.
- Sheet 1 (`Body1`): sm.u_channel_ratio was not verified.
- Sheet 1 (`Body1`): sm.tolerance_across_bends was not verified.
- Sheet 1 (`Body1`): sm.countersink_depth was not verified.
- Sheet 1 (`Body1`): sm.countersink_to_edge was not verified.
- Sheet 1 (`Body1`): sm.countersink_to_bend was not verified.
- Sheet 1 (`Body1`): sm.bend_relief_width was not verified.
- Sheet 1 (`Body1`): sm.bend_relief_depth was not verified.
- Sheet 1 (`Body1`): sm.countersink_land was not verified.
- Sheet 1 (`Body1`): sm.countersink_angle was not verified.
