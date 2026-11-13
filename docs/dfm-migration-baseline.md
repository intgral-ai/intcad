# DFM native migration: stage 1 baseline

Status: implementation and verification, 2026-11-13. The owner explicitly authorized publishing
this DFM implementation, its TEST-ONLY fixtures, contract and docs to the existing public
open-skyfall repository, and the migration branch to INTdfm. This authorization does not cover
customer STEP files, private factory data or credentials; none are in this branch.

## 1. Verified baseline

| Repository | Baseline commit | Where this stage works |
|---|---|---|
| INTdfm (`intgral-ai/INTdfm`) | `2f24830fcd84c8e3047eb6643139b94c27b8c1b6` (main, 2026-11-12) | branch `migration/pil-295-replay-contract`, head `f630dc4` at this writing |
| open-skyfall (`intgral-ai/open-skyfall-desktop`) | `1a45733ef96b53990fec8bd7794563eebc6facdb` (develop, 2026-11-10) | branch `dfm/m1-native-hole-min-diameter`, head `15e84c8a4` at this writing |

The PRD's `7bf03f6d9` is an older develop commit; current develop is `1a45733e`. The PRD and the
technical design are proposals except for the sections they mark as confirmed. This stage did
not change either document.

Task ownership checked in Linear on 2026-11-12:
- PIL-295, PIL-296 and PIL-297 are assigned to HuaiYuan Jing, status Backlog, no comments.
- Tianwei's DFM work is PIL-292 and INT-1441 (open-skyfall). This stage does not touch their
  branches; the current main checkout's uncommitted validation changes were left alone.

## 2. What moved

### 2.1 The handover contract (INTdfm, PIL-295)

`src/intdfm/replay.py` defines `intdfm.dfm-replay` version 1. One recording holds:
- the facts the geometry layer gave the rule layer (`SolidFacts`, including holes and bends);
- the declared process, material, thickness and any critical dimensions;
- the generic profile, the optional shop profile, and the gate findings;
- the traceability the old report used;
- the complete expected report.

Replay runs `rules.py` and `report.py` only: no OCP, no process, no file, no clock. A recording
from another source hash is refused. `tests/fixtures/dfm-replay/v1/` holds ten required cases
with a manifest of SHA-256 digests:

| Case | Proves |
|---|---|
| `hole_min_diameter.above` | above `0.5t` is counted, never passed |
| `hole_min_diameter.below` | `0.5` mm below `0.75` mm |
| `hole_min_diameter.boundary` | exactly `0.5t` is met |
| `hole_min_diameter.factory.below` | a TEST-ONLY factory number asks for review; unconfirmed data cannot decide |
| `hole_to_edge.below` | a `1.5` mm wall against `1.5t` |
| `hole_to_bend.below` | a `2.5` mm wall against `2.5t + r` |
| `hole_to_bend.not_applicable` | a flat part has no bend |
| `missing_thickness` | the thickness gate blocks the numeric rules with one cause |
| `holes_step_failed` | a failed hole step is reported, not silently zero |
| `shop_not_applicable` | a shop row for another material does not fall back |

Contract details, units, coordinate frame, tolerance split and commands: `docs/dfm-replay.md`
in INTdfm. Command:

```sh
python -m intdfm.replay verify tests/fixtures/dfm-replay/v1
```

The verifier fails on a missing case, a missing or altered manifest, a duplicate id, a case
renamed to another rule's id, an inconsistent gate, a non-finite fact or a changed rule source.

### 2.2 The native chain (open-skyfall, PIL-296 start)

| Layer | File | What it does |
|---|---|---|
| Facts | `crates/doc/src/sheet_dfm.rs` | measures round through holes, folded positions, edge distance and bend gaps from `SheetBody`; exact lines and arcs only. `edge_mm` counts outline, non-circular openings and pockets, never another round hole's wall (that belongs to `sm.hole_spacing`) |
| Rules | `crates/dfm-rules/` | pure Rust: validates data, chooses the number, runs the thickness gate and the three hole rules, maps to App states, writes one report; `second_gate` compares recorded and measured facts |
| App entry | `crates/engine/src/dfm.rs` | converts facts to the rule contract and calls the rules crate; no OCP, no files |
| Runner | `crates/engine/src/validation/runner.rs` | new `dfm.sheet` check kind; evidence = facts + report; `required = false` |
| Validation | `crates/engine/src/validation/profile.rs` | validates `dfm.sheet` parameters before a run |

The rule crate has no workspace dependencies. `xtask/src/layers.rs` registers it as a standalone
crate. It is in the local open-skyfall workspace now; per the PRD it moves to INTdfm when the
fork's visibility is settled.

Full PIL-296 module/rule status: `docs/dfm-rules-migration.md`.

Cross-language tests: `crates/dfm-rules/tests/intdfm_baseline.rs` loads compact projections of
the INTdfm recordings and compares status, error/reason code, measured value and limit for every
case. `crates/dfm-rules/tests/app_entry.rs` covers the App boundary cases.

### 2.3 App entry behaviour

| Situation | Result | Evidence |
|---|---|---|
| No declared thickness | thickness gate unsupported `THICKNESS_NOT_DECLARED`; numeric rules unsupported | `missing_thickness`, `missing_thickness_blocks_the_numeric_rule` |
| Declared/model conflict beyond 0.02 mm | thickness gate failed `THICKNESS_MISMATCH`; dependent rules unsupported | `declared_thickness_conflicting_with_the_model_fails_the_gate` |
| A needed fact was not measured | unsupported `HOLE_EDGE_UNCLEAR` / `NO_FLAT_TO_A_BEND`, never zero | `an_unimplemented_edge_fact_is_unsupported_not_zero` |
| No factory number | unsupported `NO_SHOP_NUMBER` | `no_factory_number_is_unsupported_and_does_not_guess` |
| Selected data has no applicable row | unsupported `COVERAGE_NOT_IN_PROFILE` | `a_factory_row_for_another_material_is_unsupported` |
| Generic value below the limit | advisory, never failed | `a_generic_number_only_advises` |
| Factory row without a confirmation | advisory | `an_unconfirmed_factory_row_still_only_advises` |
| Factory-confirmed row below the limit | failed | `a_confirmed_factory_number_can_fail` |
| Project override of a confirmed factory row | advisory, layer `project`, team-reported; it does not inherit the factory's confirmation | `a_project_override_does_not_inherit_a_factory_confirmation` |
| Boundary exactly at the generic value | checked, never passed | `a_boundary_value_is_checked_and_never_called_passed` |

The product decisions this stage deliberately does not make:
- **N-13, no declared thickness:** this stage does not adopt the model's thickness as the
  declaration. The App path with an explicit declared thickness is the tested one. The proposal
  in the PRD remains a proposal. For an imported part, the converted model's thickness is never
  used as both the declaration and the check of the original file: the declaration is a separate
  input.
- **FD-7, who confirmed a number:** the rule crate has an optional `confirmation` field and only
  factory-confirmed or production-verified rows can fail. The INTdfm format does not have this
  field yet; recordings carry none, so all recorded factory rows stay advisory. The field name
  and the person who may set it still need Tianwei's agreement.
- **Per-bend radius after conversion:** facts carry each bend's radius, but `sheet.convert` still
  writes one rule radius. A converted part with mixed radii is not accepted as equal to the
  file's part.

## 3. STEP import and conversion survey (PIL-297)

Full evidence: `docs/dfm-step-import-survey.md`. Eight public sheet-metal STEP files were
downloaded into `/tmp` only (licence notes unclear; never committed), measured with INTdfm/OCCT,
imported into SolidCraft and converted where possible. Native export -> re-import was measured
separately.

Summary (small sample, not a product success rate):
- 8/8 read as one closed, valid solid by INTdfm.
- 7/8 imported into SolidCraft as one body; 1 failed (`MotorBracketBent`, knot multiplicity).
- 4 conversions accepted, 2 refused (curved/B-spline and >90° arch), 1 partial and rejected.
- 4/4 accepted conversions had their bends and holes measured: 3 exact, `longer_side` +0.003438 %
  volume with every bend and hole exact.
- 5/5 native export -> re-import round trips were exact.
- 5/5 converted exports are still assemblies; `intdfm check` refused all with `INPUT_ASSEMBLY`.
  Fidelity is reader-level only; no converted part reached a full DFM report.

For the first three hole rules: on accepted conversions, hole diameters and positions were
preserved to 0.0000 mm, so `sm.hole_min_diameter` and `sm.hole_to_edge` are unaffected. A lost
bend is the dangerous case: `sm.hole_to_bend` becomes not applicable, which can look like a pass.
The `polpo` partial conversion accepted 1 of 6 picks with rc 0 and no warning, losing 5 bends and
2 holes; its surviving holes' nearest-bend gaps inflated to 103.64/36.72/19.82 mm. This is a
false-clear risk and needs a conversion-side guard, not a wider tolerance.

## 4. Reproduce

INTdfm:
```sh
python -m intdfm.replay verify tests/fixtures/dfm-replay/v1
python -m pytest -q tests/unit/test_replay.py
python -m pytest -q -m e2e tests/e2e/test_replay_record.py     # needs OCP
```

open-skyfall (in the branch):
```sh
cargo test -p solidcraft-dfm-rules --profile ci
cargo test -p solidcraft-engine dfm --profile ci
cargo clippy -p solidcraft-dfm-rules --all-targets --profile ci -- -D warnings
cargo xtask layers
```

## 5. Verification this run

| Check | Result |
|---|---|
| INTdfm `replay verify tests/fixtures/dfm-replay/v1` | 13 cases, exit 0 (10 required + 3 extra coverage) |
| INTdfm `pytest tests/unit/test_replay.py tests/unit/test_replay_catalog.py` | 18 passed |
| INTdfm `pytest -m e2e tests/e2e/test_replay_record.py` | 1 passed (real OCP read) |
| Rust `cargo test -p solidcraft-dfm-rules --profile ci` | 225 passed (cross-language, second gate, report, recheck, rule families, input gate, profile data and message parity) |
| Rust `cargo test -p solidcraft-doc sheet_dfm --profile ci` | 7 passed |
| Rust `cargo test -p solidcraft-engine dfm::tests --profile ci` | 18 passed (including the production recheck entry) |
| Rust `cargo test -p solidcraft-engine a_dfm_sheet_check --profile ci` | 1 passed |
| Rust `cargo xtask layers` | 25 crates, no violations |
| Rust `cargo xtask wasm` | 17 library/web crates build; `solidcraft-dfm-rules` also checked explicitly for wasm32 |
| Rust `cargo xtask assets`, `lists`, `mesh-decisions` | all pass |
| Rust clippy, rules and facts crates | clean with `-D warnings` |
| Rust clippy, engine (all targets) | clean with `-D warnings` |

## 6. Remaining differences and next tasks

1. **Three hole rules pass both gates:** each rule result matches the Python recordings, and the
   App's measured facts match the recorded facts for the same part (base, flange, multi-hole,
   opening edge and transformed models). The other catalogue rules are migrated in the rule layer
   but the App does not yet measure all their facts.
2. **Second gate tolerance:** contract schema 2 pairs by an exact complete matching. A pair must
   have position, diameter and depth within 0.001 mm; the 0.05 mm centre distance is only the
   fallback bound used to diagnose a mismatch. More than one complete matching is refused;
   a unique matching determined by diameter/depth is accepted. Differences are rounded to
   1e-6 mm before comparison. The rule threshold's 1e-6 rounding is separate.
3. **More cases:** hole-by-slot, hole spanning a bend, blind/tilted hole, countersink, perforation
   array, multiple sheet bodies with different thicknesses.
4. **App report storage:** add `findings.json`/evidence export and the `.intgral` run identity
   before the UI work. This stage stops at the structured report.
5. **Result surface:** `dfm.sheet` is a validation check kind; there is no `dfm.check` command or
   chat item yet. That is the next slice, after the UI decision.
6. **Conversion radius:** implement per-bend radius and the closed-solid gate before checking
   imported parts. This stage only measured the gap.
7. **Visibility:** the owner authorized the public push on 2026-11-13. The old "do not push"
   constraint is superseded for this branch only; it does not authorize a merge, release or the
   publication of customer STEP files, private factory data or credentials.

## 7. Open decisions for the owner

- N-13: when no thickness is declared, may the App use the sheet body's own thickness? Proposed,
  not adopted here.
- FD-7: which field records who confirmed a number, and who may set it? Proposed here as
  `confirmation` with three values.
- N-10: does a flange that ends flush with the bend tangent count as relief? The bend-relief rule
  is not part of this stage, but the fact layer will need the answer.
- Multiple sheet bodies with different thicknesses: current proposal is one check per body, each
  with its own thickness. Not settled.
