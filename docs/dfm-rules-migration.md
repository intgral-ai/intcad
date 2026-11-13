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
| `profile.py` | read/validate profile JSON, applicability, effective rows, requirements digest, gauges | **migrated (data API)** | types, validation, selection/applicability, die/bends validation, `effective`, `applying`, `requirements_sha256` and gauge tables are migrated; file loading stays at the App boundary |
| `gate.py` | input gate, thickness gate, coverage gate, reader log, shared tolerance helpers | **partial** | input gate, coverage gate, thickness gate and `same_sheet` are migrated; the second thickness measure (2V/A) and reader log stay at the App boundary |
| `rules.py` | flat test, bend gate, 15 rules, header, part facts, threshold selection | **migrated** | all 15 non-gate rules, the flat test and bend gate are migrated with recording-backed tests; header is in the report parity layer; part facts are in the parity report |
| `report.py` | report types, merge, rounding, ids, sort, counts, clean, canonical JSON | **migrated (parity layer)** | `crates/dfm-rules/src/intdfm_report.rs` reproduces the INTdfm-shaped report from pre-merge findings and metadata; 23 parity tests over the recordings. The App-facing `Report` remains separate. |
| `recheck.py` | pair two reports, marks and guard flags | **migrated** | `recheck.rs`: pairing, marks, guards, not-compared; 11 fixture pairs, byte-exact `Recheck` JSON; `engine::dfm::recheck_reports` is the production entry |
| `core.py` | order the gates and rules into one report | **migrated (rule layer)** | `catalog_findings`/`check_catalog` order all gates/rules; `dfm.sheet` calls `check_catalog` with explicit missing-fact families |
| `engine.py` fact types | the data contract between geometry and rules | **partial** | holes, bends, webs and recognition fields exist in the Rust data model; countersink, run/unrelieved facts and several solid facts are not yet used |
| `project.py`, `snapshot.py` | project text and STEP scan | **out of scope** | file/subprocess/STEP work; the App supplies the equivalent inputs |

## 2. Rules

| Rule | Rule layer | App can measure its facts? | Parity evidence |
|---|---|---|---|
| `sm.gate_input_solid` | not migrated (App gives the equivalent input-gate result) | yes, native sheet body / converted gate | App boundary tests |
| `sm.gate_sheet_thickness` | migrated | yes (`SheetBody.t` and declared thickness) | missing/conflict/equal tests |
| `sm.gate_profile_coverage` | **migrated (coverage gate)** | data only | — |
| `sm.gate_bends_found` | **migrated** | recognition facts partially | — |
| `sm.hole_min_diameter` | migrated | yes | 10 INTdfm recordings + second gate |
| `sm.hole_to_edge` | migrated | yes (cut edge, nearest point, kind, ownership) | recordings + second gate (base, flange, multi-hole, opening, transformed) |
| `sm.hole_to_bend` | migrated | yes (bend pairing, tangent gap, radius, panel association) | recordings + second gate (L-bracket, U-channel flange holes, mapped bend numbers) |
| `sm.hole_spacing` | **migrated** | App carries panel assignment, but the rule crate uses recorded flat ids | rule tests |
| `sm.inside_radius` | **migrated** | bend radius exists | — |
| `sm.min_flange` | **migrated** | no (App does not measure runs) | rule tests; App reports unsupported |
| `sm.max_bend_length` | **migrated** | bend length exists | — |
| `sm.bend_relief_present` | **migrated** | no (relief-end definition still open) | rule tests; App reports unsupported |
| `sm.min_web_parallel_bends` | **migrated** | no (App does not measure webs) | rule tests; App reports unsupported |
| `sm.u_channel_ratio` | **migrated** | no (App does not measure webs/flange lengths) | rule tests; App reports unsupported |
| `sm.bend_radius_consistent` | **migrated** | bend radii exist | — |
| `sm.tolerance_across_bends` | **migrated** | critical dimensions are carried; App does not yet collect them | rule tests |
| `sm.countersink_depth` | **migrated** | no countersink facts in the App | rule tests; App reports unsupported |
| `sm.countersink_to_edge` | **migrated** | no countersink facts | rule tests; App reports unsupported |
| `sm.countersink_to_bend` | **migrated** | no countersink facts | rule tests; App reports unsupported |

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
| Report merge/round/id/sort | migrated (parity layer) | `intdfm_report` covers merge, CPython rounding, ids, labels, sort, counts, header, bends/part and canonical JSON; App report shape stays separate |
| `header` (public-value summary) | not migrated | needs all rules |
| `part`/`bends` facts in the report | partial | webs, other openings, bend flange runs |
| `not_checked` line | migrated | exact INTdfm text restored |
| Recheck marks/guards | migrated | `recheck.rs`; 11 pairs, byte-exact `Recheck` JSON |

## 4. Recordings

Current baseline: ten required hole-rule cases plus three second-gate coverage cases. A separate
catalog set adds six recording-backed parts: countersink, spacing, mixed bend radii, missing
relief, U-channel tabs and a declared critical dimension. Together they exercise every rule
family and the pairing path. All carry schema 2 and a full expected report.

Recheck has no committed real two-report pair: its 11 pairs are TEST-ONLY transformations of a
recorded report, generated by the Python rule/report layer. A real before/after STEP pair remains
a coverage gap, not a migration blocker.

## 5. What this round does not absorb

- App extraction for every rule (countersinks, 20-place flange runs, relief ends, bend paths).
- Full UI, project memory, waivers, Spec extraction or automatic CI.
- STEP conversion fixes. PIL-297 is an investigation; converter changes need a separate decision.
- Retiring the Python reference. It stays until the full Rust catalog passes the recorded
  baseline and the team accepts the handover.
