---
name: solidcraft-analysis
description: "Use when a user asks how an Intcad part behaves under load (stress, deflection, will it bend, is it strong enough) and you run inspect.stress_estimate through the MCP server."
---

# Stress estimates

`inspect.stress_estimate` meshes one body (gmsh) and solves it (CalculiX) on the user's computer: linear static, small deflection, one load case. It is an estimate, never a verdict: report it as the reply's `how_to_report` says. Worked setups: `skill_reference {"skill":"solidcraft-analysis"}`.

## Workflow
1. **Faces:** `execute model.faces {}` gives each face's name, index, centroid and normal. Faces the user selected come with their message. Name or index a face; a point `[x,y,z]` must lie within 1 mm of it. One half of a hole takes the whole hole.
2. **Setup, in numbers first:**
   - **Supports.** `fixed` holds a face rigidly, stiffer than a real bolt or clamp, so the stress right at it is not to be trusted (lead with `peak_away`). A bolted hole: fix the hole wall. `roller` holds a flat face facing X, Y or Z only across itself; `symmetry` is the same on a cut plane.
   - **Loads.** `force: [x,y,z]` in N, the total on the faces; or `direction` (any length) and `magnitude` in N. A mass: kg × 9.81 N along the user's "down"; ask when down is unclear. A pressure takes its unit (`"2 bar"`); a bare number is MPa, and positive pushes into the part.
   - **Material.** A grade by name (`6061-T6`, `6082-T6`, `5052-H32`, `S235`, `S355`, `304`, `304L`) brings sourced numbers at the standard's minimum yield. No grade or temper given ("steel", "6061"): ask the user; never take one from an example. Anything else: the three numbers with their real source. Never invent a source. Plastics are not in the table: a printed part depends on how it was printed.
   - **Probes.** A value at a named point: `probes` (reference: a strain gauge).
3. **`dry_run: true` first.** It checks the setup, refuses a part that could still slide or turn (and says what to add), and draws the supports and loads on the model. Tell the user what it shows, and fix the faces they correct.
4. **Run** without `dry_run`. The user allows it in the chat; it takes seconds to minutes, and Stop ends it.

## Reporting
- Follow `how_to_report`. Warnings and the `equilibrium` line come before any number.
- If `peak_at_support` is set, lead with `peak_away` and its fraction of yield; the support's value depends on the mesh.
- Give each number's mesh check (settled or still changing, with its %), and the material and its source.
- Deflection: `runs[].max_deflection_mm`. To show it: `inspect.stress_estimate.show {"show":"deflection"}`.
- "Is it strong enough?": answer with the numbers (where the stress peaks, about how large, what fraction of yield) and what the estimate leaves out: real bolts and welds, fatigue, buckling, impact, temperature. Do not call it safe, unsafe or fine, give a factor, or announce that you will not judge.

## Changes
For another thickness, load or shape: change the design (`set_parameter` for a user parameter, `parameters.change` for a sketch or feature dimension; see `solidcraft-editing`), then dry-run the same setup again (faces can move or renumber) and run it. Never scale numbers from an earlier run. `thickness` is the shells' alone; it does not change the model.

## Not available yet
A pin or hinge about a hole, a bolt pressing on a hole wall, gravity, a roller on a sloped face, several load cases, and colour maps of principal stress (a `screenshot` shows von Mises). Say so; never stand one in for another silently.
