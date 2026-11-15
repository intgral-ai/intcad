# PIL-291 integration record

Local integration branch: `coord/pil-291-integration` in `worktrees/accept`, built with
`CARGO_TARGET_DIR=/home/hyjing/Code/intgral/intcad/.targets/accept`.

Merged for the first desktop round (2026-11-14):

| PR | Branch | Head | Content |
|---|---|---|---|
| #124 | `pil-299-hole-facts-audit` | `1de0329ec` | hole-facts audit evidence |
| #125 | `pil-306-flat-overlap` | `c26b5db8f` | flat self-overlap geometry + adapter |
| #126 | `pil-298-project-create` | `f9f8fa43b` | project folder `.intcad/`, create/attach/inspect, New Project UI |
| #135 | `pil-301-project-config` | `e6a7d06af` | factory data copy, freeze, settings, overrides, `.intcad` data |
| #131 | `pil-304-viewport-marks` | `84c366271` | viewport marks, labels, freshness |
| #137 | `pil-291/c` | `d3c158759` | run store, findings, freshness (phase 1; host wiring in progress) |

Merge result: `c3e17d599` (before #131's i18n fix; rebuilt after merge of all five).

Rename decision: `.intcad/` is the project data directory. No migration and no dual-directory
code: the feature never shipped and no `.intgral` directory exists on the machine. The only
remaining `.intgral` in branches is `docs/dfm-migration-baseline.md`, which #126 changes; the
other hit is the unrelated `intgral-ai` org id.

Checks: all six PR heads are green on their own CI at the SHAs above (see the PR pages); #137's
clippy fix and phase 2 are still in flight.

## Desktop round 1 (project + factory data, real window, offline clean copy)

Built from `coord/pil-291-integration` (`c3e17d599`, merge of #124/#125/#126/#135/#131) into
`.targets/accept`. App run under Xvfb, X11 window system, control channel.

1. `project.create {name:"Factory Acceptance"}` → only `.intcad/` exists (no `.intgral/`); design saved.
2. `project.data_set {kind:"factory", path: TEST-ONLY-shop-two-tier.json}` → whole file copied to
   `.intcad/data/d182cd8f-…json`; `ci.toml` records name/version/SHA-256
   (`d182cd8f41a5…d883`); `dataAvailable: true`.
3. `project.settings_set {material:"5052-H32"}` (no declared thickness).
4. Copied the project folder to `/tmp/opencode/clean301`, started the app with a fresh
   `HOME`/`XDG_DATA_HOME` **inside `unshare -rn` (no network; `curl` fails)**, opened the copied
   design, and `project.inspect` still reported the project id `967d1912-…`, material `5052-H32`,
   the data copy and its SHA.
5. `project.settings_set {thickness_mm: 2.0}` → `project.inspect` reports `thicknessMm: 2.0`.

The tier-2 hit (2.0 mm → `sm.hole_min_diameter` 3.5) still needs the manual `dfm.check` run
(PIL-302 host wiring, in progress). Evidence images: `acc301-project.png`,
`acc301-clean-offline.png` (not committed).

## Desktop round 2 (manual DFM run in a clean offline copy, .intcad)

Integration head after #126/#135/#131/#137/#138 + the `.intcad` rename. Built once into
`.targets/accept`; app under Xvfb + control channel.

1. Project "Hole Tier": `project.data_set` copied `TEST-ONLY-shop-two-tier.json` into
   `.intcad/data/d182cd8f-…json` (SHA unchanged); material 5052-H32; no declared thickness.
2. Model: box 60×40×2.0 → `sheet.convert` → `sheet.perforate` Ø3 pitch 10 margin 5.
3. Project copied to `/tmp/opencode/tierclean`; app relaunched inside `unshare -rn` (offline)
   with a fresh HOME/XDG, design opened, `dfm.check`:
   `run-507e9b2ee47741b3-88cca0805d2286fb`, outcome `current`.
   Per-hole `sm.hole_min_diameter`: measured 3.0 mm, requirement **3.5 mm** (factory tier 2),
   `factory_confirmed` → `failed`; `sm.hole_to_edge` 5.0 = 5.0 → `passed`.
   Files: `.intcad/checks/runs/<run>/{run,profile,report,findings}.json`, `summary.md`, `latest.json`.
4. Added a second sheet body at 1.2 mm (box + convert + perforate) and ran again:
   `run-f30d55a54cdf4d87-574b3de18ab18f3d`, sheets 2:
   sheet 0 (2.0 mm) requirement 3.5 → failed; sheet 1 (1.2 mm) requirement 2.0 → passed.
   This proves per-body thickness/tier selection.
5. Killed and relaunched offline; `doc.open` + `inspect.dfm` read the same run, rule rows
   (`ruleId`, failed, 21) and both thresholds. Only `.intcad/` exists.
6. `freshness` after a cross-session reopen is `unknown` (no live candidate digest was supplied);
   the card must pass the live digest or the host must compare it. Recorded as a gap.

## Final integrated checks (host level)

On `coord/pil-291-integration` (all eight heads, including #141):

```
cargo test -p solidcraft-engine --lib dfm            → 55 passed
cargo test -p solidcraft-ui-egui --lib dfm_runtime   → 8 passed
cargo test -p solidcraft-ui-egui --lib agent_chat::web::tests → 4 passed
```

The web test includes the row → viewport selected-key and label → same row round trip; the
trigger tests cover the 1/0/1/1/0/0 run counts; the engine tests cover the late-run supersede
interleaving (including the check-then-publish race).

## Real run with app.flat_no_overlap, and rapid submissions

Offline clean copy, real desktop control channel:

- run `run-86bc2c7e6a364a8b-64cbff705b349d3a` contains `app.flat_no_overlap` as a rule row
  (`passed`, two sheets) and one App finding per sheet. The failed-overlap case is covered by
  the doc geometry tests plus the 12.5 mm² mapping test, not by a desktop sample.
- Three `dfm.check` calls in quick succession: the first returned `replacesRunning: false`, the
  next two `true`; the first run was recorded `cancelled`, the last run is `latest`. So a newer
  submission replaces the running check and an older one does not keep `latest`. A genuinely
  late *uncancelled* return (recorded `superseded`) is covered by the engine tests only.
