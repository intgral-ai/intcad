# Stress estimate: worked setups

Each setup is an `execute` of `inspect.stress_estimate`. Send it once with `"dry_run": true`, show the user the supports and loads it draws, then send it again without. A face is a name or index from `model.faces`, or a point within 1 mm of it: a flat face's centroid works, a hole's or a curved face's does not (it lies off the face). Names such as `rim` below stand for the names `model.faces` gives.

## A cantilever strip (the app's own check)
A 100 × 20 × 1.5 mm aluminium strip along X, clamped at x = 0, 10 N down at the free end.
```json
{"command": "inspect.stress_estimate", "params": {
  "supports": [{"face": [0, 10, 0.75]}],
  "loads": [{"face": [100, 10, 0.75], "force": [0, 0, -10]}],
  "material": "6061-T6",
  "dry_run": true}}
```
Then without `dry_run`, expect: the highest stress at the clamp (`peak_at_support`); `peak_away` within about 3 % of beam theory, σ = 6 F (L − x) / (b t²) at its `x`; the tip deflection a few % under F L³ / (3 E I), because a wide strip is stiffer than a beam.

## A bracket bolted through two holes
Fix the wall of each hole by name (either half of a hole takes the whole hole). Hang 15 kg from the end flange's face: 15 × 9.81 = 147.15 N along the user's "down" (here −Z).
```json
{"supports": [{"face": "hole1"}, {"face": "hole2"}],
 "loads": [{"face": "flange_end", "force": [0, 0, -147.15]}],
 "material": "6061-T6"}
```
A fixed hole wall is a rigid bolt: stress at the holes depends on the mesh. Report `peak_away`, and say the hole values are idealised.

What each hole holds is in `equilibrium.supports`, one entry per support in the order given: `force_n` (N, on the part; the bolt takes the opposite), and `moment_nmm` (N·mm) about `about_mm`, the middle of the hole's held nodes. Quote its `line`. A rigid wall shares the load as a stiff bracket would, so read the split between the holes as an estimate too. `equilibrium` also says whether the moments about the part's middle balance (`moment_mismatch_percent`); over 1 % is a warning that the result is suspect.

## A lid under pressure
A plate on a tank at 2 bar, its rim held. The pressure pushes into the part from the tank side.
```json
{"supports": [{"face": "rim"}],
 "loads": [{"face": "inside", "pressure": "2 bar"}],
 "material": "5052-H32",
 "show": "deflection"}
```
A fixed rim is a fully clamped edge; a bolted or simply resting rim bows more. Say which was modelled. The bow is `runs[].max_deflection_mm`.

## A half model on a symmetry plane
Cut a symmetric part in half and hold the cut face with `symmetry`: the cut must be flat and face X, Y or Z. Halve the loads that cross the cut.
```json
{"supports": [{"face": [0, 5, 0.75]}, {"face": [50, 10, 0.75], "type": "symmetry"}],
 "loads": [{"face": [100, 5, 0.75], "force": [0, 0, -5]}],
 "material": "6061-T6"}
```

## The stress and strain at a point (a strain gauge)
Name the points with `probes`. Give `direction` for a gauge's axis: `along` then reads the stress and strain a gauge laid that way would. Here a gauge along the strip on its top face at mid-span, and a point on its side face read without a direction.
```json
{"supports": [{"face": [0, 10, 0.75]}],
 "loads": [{"face": [100, 10, 0.75], "force": [0, 0, -10]}],
 "material": "6061-T6",
 "probes": [{"at": [50, 10, 1.5], "direction": [1, 0, 0], "name": "gauge 1"},
            {"at": [20, 0, 0.75], "name": "edge"}]}
```
Each probe gives `principal_stress_mpa` (positive in tension, negative in compression), `strain_microstrain` (µε; the shears as engineering γ), `along`, and its own `mesh_check`. A point up to 1 mm outside the part is read at its nearest point (`outside_part_mm`); farther, the reply refuses it, or names it in `warnings` when no element is in reach.

## Your own material
Give all four numbers and where they come from; the reply repeats the source.
```json
{"material": {"name": "PETG, printed flat", "youngs_modulus": 2000, "poisson_ratio": 0.38,
  "yield_strength": 45, "source": "the filament maker's datasheet, tested flat; printed layers are weaker across"}}
```

## Refusals and what they mean
- "free to slide / turn …, add …": the supports leave a rigid motion; add the support it names.
- "is also held by a support": a load on a held face does nothing; move it.
- "this body is Aluminum, so say which grade": the body's material is generic; pick a grade or give numbers.
- "the finer mesh would need about … GB": the mesh check was skipped; say the numbers' settling is unknown.
- "`…` is not a setting of the estimate": the estimate does not take it (a remote force, a moment, load cases, gravity, bolts, contact); tell the user what it cannot model rather than dropping it, and run what it can.
