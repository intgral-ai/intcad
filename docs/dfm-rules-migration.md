# DFM rule-layer migration matrix (PIL-296)

This is the working inventory of the INTdfm OCP-free rule layer and its Rust translation. It is
deliberately separate from the App's geometry extraction: "the Rust rule exists" and "the App
can measure every fact that rule needs" are different statements.

Baseline: INTdfm `2f24830fcd84c8e3047eb6643139b94c27b8c1b6` (contract schema 2 at `79c5682`).
Rust crate: `crates/dfm-rules` (local branch only; not pushed to the public fork).

Status words:
- **Migrated**: Rust code exists and has a parity or hand-calculated test.
- **In progress**: being implemented in this round; not yet accepted.
- **Not migrated**: Python reference only.
- **Out of scope**: needs OCP, a file, a subprocess, the model or the App, and is not part of the
  rule crate.

## 1. Modules

| Python module | Job | Rust status | Notes |
|---|---|---|---|
| `profile.py` | read/validate profile JSON, applicability, effective rows, requirements digest, gauges | **partial** | `Profile`/`Requirement`/`Applies`/`Die`/`Limit` types, unknown-key rejection, finite values, thickness gate selection and applicability are migrated; the exact `requirements_sha256` canonical digest, gauge tables and all profile-format error texts are not |
| `gate.py` | input gate, thickness gate, coverage gate, reader log, shared tolerance helpers | **partial** | thickness gate and `same_sheet` are migrated; input gate, coverage gate, reader log, gauge messages and scan/reader facts are not |
| `rules.py` | flat test, bend gate, 15 rules, header, part facts, threshold selection | **partial** | `sm.hole_min_diameter`, `sm.hole_to_edge`, `sm.hole_to_bend` are migrated and replay-tested; the other bend, tolerance and countersink rules, bend gate, header and part facts are not yet migrated |
| `report.py` | report types, merge, rounding, ids, sort, counts, clean, canonical JSON | **partial** | a Rust report exists for the App path; it does not yet reproduce INTdfm's merge, labels, finding ids, header, bends/part facts or canonical bytes |
| `recheck.py` | pair two reports, marks and guard flags | **not migrated** | needs the full report and part facts first |
| `core.py` | order the gates and rules into one report | **partial** | the App entry orders the thickness gate and three hole rules; the full gate/rule sequence is not migrated |
| `engine.py` fact types | the data contract between geometry and rules | **partial** | holes, bends, webs and recognition fields exist in the Rust data model; countersink, run/unrelieved facts and several solid facts are not yet used |
| `project.py`, `snapshot.py` | project text and STEP scan | **out of scope** | file/subprocess/STEP work; the App supplies the equivalent inputs |

## 2. Rules

| Rule | Rule layer | App can measure its facts? | Parity evidence |
|---|---|---|---|
| `sm.gate_input_solid` | not migrated (App gives the equivalent input-gate result) | yes, native sheet body / converted gate | App boundary tests |
| `sm.gate_sheet_thickness` | migrated | yes (`SheetBody.t` and declared thickness) | missing/conflict/equal tests |
| `sm.gate_profile_coverage` | not migrated | data only | — |
| `sm.gate_bends_found` | not migrated | recognition facts partially | — |
| `sm.hole_min_diameter` | migrated | yes | 10 INTdfm recordings + second gate |
| `sm.hole_to_edge` | migrated | yes (cut edge, nearest point, kind, ownership) | recordings + second gate (base, flange, multi-hole, opening, transformed) |
| `sm.hole_to_bend` | migrated | yes (bend pairing, tangent gap, radius, panel association) | recordings + second gate (L-bracket, U-channel flange holes, mapped bend numbers) |
| `sm.hole_spacing` | not migrated | no (hole-to-hole wall is separate from edge) | — |
| `sm.inside_radius` | not migrated | bend radius exists | — |
| `sm.min_flange` | not migrated | not yet (runs at 20 places, both flats) | — |
| `sm.max_bend_length` | not migrated | bend length exists | — |
| `sm.bend_relief_present` | not migrated | no (relief-end definition still open) | — |
| `sm.min_web_parallel_bends` | not migrated | webs partially | — |
| `sm.u_channel_ratio` | not migrated | webs/flange lengths partially | — |
| `sm.bend_radius_consistent` | not migrated | bend radii exist | — |
| `sm.tolerance_across_bends` | not migrated | critical dimensions exist; bend-path logic missing | — |
| `sm.countersink_depth` | not migrated | no countersink facts in the App | — |
| `sm.countersink_to_edge` | not migrated | no countersink facts | — |
| `sm.countersink_to_bend` | not migrated | no countersink facts | — |

## 3. Data, gates and report pieces

| Piece | Status | What is missing |
|---|---|---|
| Profile unknown keys / finite values / required keys | migrated | exact INTdfm error text and every format rule |
| Shop-over-generic replacement | migrated | `effective` exposed as a standalone helper |
| Applicability by process/material/temper/thickness | migrated | parity tests for every gap reason |
| Strictest-row selection | migrated for the three hole rules | `sm.min_flange` die rows |
| Project override provenance | migrated | FD-7 field is still a proposal; overrides are team-reported |
| Thickness gate | migrated | gauge message table and both-measure 2V/A check |
| Coverage gate | not migrated | profile gap and missing-rule messages |
| Input gate | not migrated | scan/reader facts; App entry supplies its own result |
| Bend gate | not migrated | recognition facts and messages |
| Report merge/round/id/sort | in progress | a parity layer is being added; existing App report shape stays |
| `header` (public-value summary) | not migrated | needs all rules |
| `part`/`bends` facts in the report | partial | webs, other openings, bend flange runs |
| `not_checked` line | migrated | text matches |
| Recheck marks/guards | not migrated | all of `recheck.py` |

## 4. Recordings

Current baseline: ten cases, all one hole each; they prove the three hole rules and the App
boundary semantics. They do **not** exercise the bend, tolerance, countersink or recheck rules,
and they do not exercise pairing with more than one hole.

Before PIL-296 can be called complete for the full rule catalog, add recordings for:
- a flat multi-hole plate (pairing, spacing, edge ownership);
- a U-channel and an L-bracket with holes on base and flange (bend rules and association);
- a part with a full bend-rule profile (inside radius, flange, length, relief, web, U ratio);
- a countersunk part;
- two reports of the same part before/after a change (recheck marks and guards).

Each recording must carry the schema-2 `comparison` block and a full expected report.

## 5. What this round does not absorb

- App extraction for every rule (countersinks, 20-place flange runs, relief ends, bend paths).
- Full UI, project memory, waivers, Spec extraction or automatic CI.
- STEP conversion fixes. PIL-297 is an investigation; converter changes need a separate decision.
- Retiring the Python reference. It stays until the full Rust catalog passes the recorded
  baseline and the team accepts the handover.
