# INT-1454 — desktop editor and DFM boundary: IntCAD develop vs PR #145

Read-only trace, 2026-11-16. No builds, no tracked-file changes.

| Ref | SHA | Tree used |
|---|---|---|
| A — latest develop | `02a786d47180ffa44a99f0508749c1c11b383c0d` | `git archive` → `/tmp/opencode/int1454-v2/src-a` |
| B — PR #145 head | `f45d977481614fad850c7f9ca67e48e6e7956e5f` | `git archive` → `/tmp/opencode/int1454-v2/src-b` |
| DFM slice in A | #131 `18366bbf0`, #157 `183cf7836`, #158 `4eb71a7a6` | — |

Line numbers are for the archived trees at those commits.

---

## 0. One-paragraph answer

The desktop editor is the same program in both refs: `apps/solidcraft` owns the window, the
control channel and the native Assistant web view; `crates/ui-egui` owns everything the user sees;
`crates/engine` owns commands and state. A draws that UI with an **in-crate** `crates/ui-egui/src/ui_kit/`
(66 files) and a hand-rolled `shell.rs`/`frame.rs` panel layout. B replaces it with the external
**`solidcraft-ui-kit`** crate (plus 15 new siblings: `ui-text`, `theme`, `look`, `ui-registry`,
`viewport`, `viewport-egui`, `viewport-gpu`, `control`, `control-egui`, `format`, `color`, …) and a
retained panel system (`panel_runtime`) — that swap is the whole cost of route B, and it is
**not** where the DFM risk is. The DFM two-way selection lives entirely in `crates/ui-egui`
(`dfm_overlay.rs`, `dfm_review.rs`, `dfm_chat.rs`, `right_rail.rs`, `agent_chat/web.rs`) with one
authoritative selection in `ViewportState.dfm`. None of that exists at B: #145 forked before the
DFM slice merged, so B has **no** `dfm.check`/`inspect.dfm`, no run store, no findings file, no
anchors, no marks and no Review rail. Route B therefore pays the kit swap *and* re-ports the
whole DFM stack; route A pays only UI requests against a system that already works.

---

## 1. Entry chain into `SolveApp`, both refs

### A (`02a786d`)

```
apps/solidcraft/src/main.rs:1226  eframe::run_native(…)
  └ :1241  SolveApp::new(Session::default(), services(cc, &dialog_folder))
      └ crates/ui-egui/src/lib.rs:825  SolveApp::new       (fields, no kit call)
  └ App { solve: SolveApp, assistant_web: AssistantWeb }   main.rs:89, :106
impl eframe::App for App                                    main.rs:~195
  ├ update/logic
  │   └ self.solve.logic(ctx)                               main.rs:252
  │       └ lib.rs:2968  SolveApp::logic  (shortcuts, control drain)
  ├ self.assistant_web.logic(&mut self.solve, ctx)          main.rs:254
  ├ ui(ui, frame)                                           main.rs:~288
  │   ├ content_ui(&mut self.solve, ui)                     main.rs:478
  │   │   └ lib.rs:3123  SolveApp::ui
  │   │       └ lib.rs:3174  SolveApp::cad_ui
  │   │           ├ :3195  toolbar::app_bar(self, ui)  ← crate::ui_kit::Container / app_bar (in-crate)
  │   │           ├ :3222  status_bar::show
  │   │           ├ :3224  right_rail::show              right_rail.rs:163
  │   │           ├ :3226  dfm_review::poll
  │   │           ├ :3227  dfm_review::drag_hint
  │   │           ├ :3228  dfm_review::release
  │   │           ├ :3241  agent_chat::show(self, ui)    panel.rs:113
  │   │           ├ :3314  Container(…Viewport).central() → self.show_workspace(ui)
  │   │           │         └ viewport::show             viewport.rs:4729
  │   │           └ :3339ff dialogs, context menu, prefs…
  │   └ self.assistant_web.tick(&mut self.solve, ui.ctx(), frame)  main.rs:291
  │        └ assistant_web.rs:479  web::placement(solve, ctx)   → :503 view.set_bounds(place(at))
```

### B (`f45d9774`)

```
apps/solidcraft/src/main.rs:1224  eframe::run_native
  └ :1239  SolveApp::new
      └ crates/ui-egui/src/lib.rs:911  SolveApp::new
          :920 i18n::install()
          :922 solidcraft_look::install_tables()          ← look, not kit
          :924 workspace::install_tabs()
          :926 navigation_slots::install()
impl eframe::App for App                                    main.rs:195
  ├ logic(ctx, frame)                                       main.rs:196 (eframe 0.37 split API)
  │   └ self.solve.logic(ctx)  → lib.rs:3385
  │        :3391 panel_runtime::tick_app(self, ctx)         ← retained panel shell
  ├ ui(ui, frame) → content_ui → SolveApp::ui  lib.rs:3604
  │    :3614 ui_kit::set_dialog_mnemonics(…)                ← FIRST executed ui-kit call
  │    :3620 self.cad_ui(ui, …) → lib.rs:~3650
  │        toolbar::app_bar(self, ui)   toolbar.rs:201
  │          :203 Container::new(…)  :24x window_chrome::TitleBar { … }.show
  │          :8xx ribbon::tabs → TabAction::Run { id } => app.start(&id)
  │        viewport::show → navigation::handle(caps=MODEL)
  └ self.assistant_web.tick(…)                              main.rs:~289
```

### The first call that forces the external kit at B

Three forcing points, in increasing order of "really external":

1. **Dependency + re-export (compile time):** `crates/ui-egui/Cargo.toml:22`
   `solidcraft-ui-kit = { workspace = true }`, and `crates/ui-egui/src/lib.rs:344`
   `pub use solidcraft_ui_kit as ui_kit;`. At A the same name is `lib.rs:317 pub mod ui_kit;`
   — 66 local files. Every `crate::ui_kit::…` in 226 A-side files silently changes owner (`grep -rl ui_kit crates/ui-egui/src`).
2. **First kit type in a struct:** `lib.rs:599 pub(crate) viewport_status: ui_kit::StatusPresentation`
   (A `lib.rs:535`, same type name, local crate).
3. **First kit call executed in a frame:** `lib.rs:3614 ui_kit::set_dialog_mnemonics(…)`, before
   anything is drawn; the first kit **component** drawn is the application bar
   (`toolbar.rs:201-203` → `app_bar::AppBar`, `window_chrome::TitleBar`).

So: route A keeps the in-crate kit at zero licence/API risk; route B's first forced edge is a
Cargo dependency, not a runtime decision — it cannot be feature-flagged away later without
rewriting the call sites.

---

## 2. Toolbar → engine command (`SolveApp::run`)

```
button / menu row                         command id                engine
──────────────────────────────────────────────────────────────────────────────
A: toolbar::grid_action  toolbar.rs:2821  RowAction::Command(id)
   A: toolbar::Action::run  toolbar.rs:707  Self::Command(c) => app.start(c)
   A: context menu / shortcut             app.start(id)             lib.rs:1603
        └ start_at → start_with_variant_memory  lib.rs:1647/1662
             direct command  → app.run(id, params)      lib.rs:1235
             dialog command  → self.dialog / self.tool  (params collected, then run)
B: ribbon::tabs → TabAction::Run { id }   toolbar.rs:873  app.start(&id)   [same start/run]
```

`SolveApp::run` (`lib.rs:1235`) is the single host funnel: panel commands first
(`solidcraft_engine::panels::commands::is_command`), file-operation guards, then
`self.session.execute(id, &params)` (`lib.rs:1272`), then `command_completed` host effects
(`lib.rs:1304`). Commands never open dialogs on this path.

Command identity/schema: `crates/engine/src/cmd/*.rs` `CommandSpec::new(id, title, checked!(id, fn))`
with `.contract(&TypedContract::<Params>::new())`, `.output(output_schema_for::<Reply>)` and a
`.params("…")` prose line that *is* the documented schema today
(`crates/engine/src/cmd/dfm_check.rs:24-55`; `crates/engine/src/schema/dfm.rs`).
DFM ids: `dfm.check`, `dfm.cancel`, `dfm.status`, `inspect.dfm` (`dfm_check.rs:24,33,40,47`).

---

## 3. Camera, picking, pointer capture (identical architecture, different shell)

One camera path in both:

* `crates/ui-egui/src/navigation.rs:548` (A) / `:676` (B) — `navigation::handle(app, ui, rect, resp, caps)`.
  Called from `viewport.rs:4741` (`NavCaps::MODEL`) and from the Render preview. The module doc
  (`navigation.rs:1-10`) states every canvas must call it; a guard test
  (`navigation.rs:1925 hand_rolled_camera_input_is_rejected`) fails hand-rolled wheel/drag reads.
* Input gates, in order: `resp.contains_pointer()` → `ui.rect_contains_pointer(rect)` →
  `!shell::covered(app, p)` (`shell.rs:188` A / `:100` B) → axis view / view-angle readout →
  floating windows and menus own input (`navigation.rs:566-580`).
* Pointer capture across floating cards: `cad_ui` computes `continuing_navigation` from
  `press_origin` + `pointer.(middle|secondary)_down` (`lib.rs:3276-3282`) and disables the viewport
  only when a *new* gesture starts under a card.
* Picking is **CPU**, no depth/ID buffer: `viewport.rs:2585 pick` → `pick_hits`, cached per
  (cursor, camera, revision, visibility, prefs) at `viewport.rs:2541 pick_cached` keyed by
  `last_pick`. `pick_all`/`pick_through` (`:2604`, `:2620`) serve Select Other and see-through.
  Projection is `Proj` (`viewport.rs:379`) with `to_screen` (`:386`); no GPU readback is used for
  selection anywhere in this path.

B adds `viewport_drag_layer.rs` and `crates/viewport{,-egui,-gpu}` extraction, but the camera math
and hit test remain in `ui-egui` — the extraction is scene/GPU data, not input arbitration.

---

## 4. Panel ↔ canvas contention (who gets the click first)

A:

1. egui layer order (Foreground menus/dialogs > floating cards > edge panels > central viewport).
2. `shell::covered_rects/covered` (`shell.rs:155/188`) refuses hover and presses over any registered
   shell rect.
3. Docked panels are separate `Container::edge` allocations before the central viewport
   (`lib.rs:3224` rail, `:3314` viewport); they physically shrink `ui.available_rect_before_wrap`,
   so the view never sees the pointer there.
4. Floating: `Container::for_layout(layout, Surface::Panel).floating()` (`agent_chat/panel.rs:145`)
   draws cards over the model; input is shielded by (2) plus `ui.disable()` on the viewport child.
5. **Native WebView** sits *above* all egui painting. It therefore receives the click, wheel and
   drag itself; egui never sees them. The app compensates by (a) hiding the view whenever a
   floating layer or the picture viewer covers its rect (`agent_chat/web.rs:116-146 placement`),
   (b) leaving a resize strip free at the panel's inner edge (`web::rail()`, `panel.rs:258-283`),
   (c) taking the keyboard back on a press outside the panel (`panel.rs:96-110 owns_keyboard_input`),
   (d) remembering the last pointer pos for a drag released over the view (`dfm_review.rs:193-227`).

B keeps the same Assistant mechanism (`agent_chat/panel.rs`, `agent_chat/web.rs` at nearly the same
line numbers) but replaces the docked/floating split with `panel_runtime` (`tick_app` in `logic`,
`panel_runtime/interaction.rs` pointer-capture candidates, `layout.rs` resolved host rects,
`production.rs:576 place_assistant`). The kit `Container` also publishes native host regions
(`ui_kit::Container::host_window_regions`, used in B `lib.rs:3711/3739`).

**Highest-risk boundary overall:** the native WebView over an egui-owned panel. Two UI systems
share one window with no common event loop: positioning (`set_bounds`), z-order, keyboard
ownership and drag-release positions are all hand-bridged, and a mis-sized or stale rect gives an
apparently dead panel. A has two post-#145 fixes here that B does not
(`apps/solidcraft/src/assistant_web.rs` diff: `latest` map-time correction, GTK
`move_/resize` sync — the INT-1472 fix, plus `assistant_web.rs:479/503`).

---

## 5. Assistant native host vs egui panel

* **Owned by the app, not the UI crate:** `apps/solidcraft/src/assistant_web.rs` (1100 lines) —
  WebKitGTK child of the X11 window on Linux, WebView2 child on Windows, wry view on macOS,
  `include_bytes!` of `packages/assistant-web/dist/*` (`assistant_web.rs:67-74`), CSP
  (`:42-44`), GTK turns per frame (`:58-70`).
* **Owned by `ui-egui`:** the rectangle. `agent_chat/panel.rs:113 show` picks the docked edge or the
  floating card and stores `panel_rect` (`:174`, `:287`); `agent_chat/web.rs:108 place` records
  `(rect, layer)`, `:116 placement` returns the rect unless covered; `app.assistant_web.tick`
  applies it. Open/minimised/side/width and the drop target are `SolveApp.agent_chat` fields.
* **Bridge:** `agent_chat/web.rs`. Page→host: `act` (`:590`) parses `{"type":…}` with an explicit
  allow-list (doc comment `:575-589`); host→page: a single JSON state built each frame
  (`:333 dfm{display,selected}`, `:258 "kind":"check"` card). Pages come from
  `packages/assistant-web/src/App.tsx` / `host.ts`.
* Selection description and keyboard ownership are host-side only (`panel.rs:96`, `web.rs:94
  selection`).

---

## 6. DFM two-way selection in A — full chain

### 6.1 Where the data comes from

```
project dir: .intcad/checks/runs/<run-id>/findings.json   (deterministic, no times)
             .intcad/checks/runs/<run-id>/report.json     (ProjectReport)
             .intcad/checks/latest.json                    (published only when complete)
```
`crates/engine/src/dfm/run.rs:1-8` (module doc), `:369-380 write_files`, `:460 load_findings`,
`:450 latest`, `:35 const LATEST`. Identity fields:
`RunFindings { schema_version, run_id, candidate_digest, profile_digest, config_digest,
model_version, project_id, source, findings, rules, not_checked }` (`dfm/findings.rs:166-184`).

Read path (one command, one truth):

```
inspect.dfm  crates/engine/src/cmd/dfm_check.rs:47 → :90 inspect()
  └ dfm/service.rs:322 inspect(session, run?, model_digest?)
      :347-349 load_meta / load_findings / load_summary
      :346-355 current_model = caller digest OR (meta.source_revision == live revision ? candidate : None)
      :356    freshness(meta, current_model, live_config)     dfm/run.rs:159
      :367    freshness → "current" | "staleModel" | "staleConfig" | "staleBoth" | "unknown"
      :368-…  findings: Some(RunFindings as JSON)
```

Freshness caveat worth pricing: `run.rs:160` returns `Unknown` unless **both** model and config
identities are known; the UI calls `inspect.dfm` with `{}` (`dfm_chat.rs:284`), so
**any model edit makes the next read `unknown`, not `staleModel`** — and no caller passes
`modelDigest` anywhere in the tree (grep: only the doc string `cmd/dfm_check.rs:53`).

### 6.2 `(run_id, finding_id)` — single source of truth

```
                  ┌───────────────────────────────────────────────┐
                  │ ViewportState.dfm : DfmMarksState             │
                  │   run_id, candidate_digest, freshness,        │  viewport.rs:76
                  │   selected: Option<finding_id>, display,      │  dfm_overlay.rs:187-195
                  │   marks, locate                               │
                  └───────────────────────────────────────────────┘
   writers ↑                                        ↓ readers
 Review row            dfm_review.rs:412-413      dfm_overlay::show   (draw + hit test)
 Assistant "finding"   web.rs:601-607             dfm_review::findings selected? :385/408
 page display mode     web.rs:608-616             web.rs:970 selected_finding → page JSON
 Review card adopt     dfm_review.rs:168-173 → dfm_chat.rs:291-305 show_marks → load_dfm_run
```
Key API: `ViewportState::load_dfm_run` (`viewport.rs:336`), `select_dfm_finding` (`:355`),
`set_dfm_freshness` (`:343`), `set_dfm_display` (`:349`), `sync_dfm_marks` (`:366`); state machine in
`dfm_overlay.rs` `load:214`, `set_freshness:228`, `selected_key:252`, `visible_marks:265`,
`select:285`, `locate_target:300`.

`app.dfm_chat.selected: Option<(run, id)>` (`dfm_chat.rs:46`) is a **mirror** used for (a) the page
state when the viewport holds no current selection, (b) the run-scoped validation in
`dfm_chat::select` (`:329-339`). It is deliberately not authoritative; `dfm_review.rs:385` and
`web.rs:970` both try the viewport first and fall back to it. This is the one place a mismatch can
show: the page validates `(runId,id)` against `dfm_chat.card` but then calls
`select_dfm_finding(Some(id))`, which selects **by id inside the viewport's own run** — if the
Review panel loaded run X and the page card is run Y, a same-id finding silently selects X's.

### 6.3 Geometry, projection, labels, hit test, tint

* Anchor/faces/outline are contract fields on each finding (`dfm_overlay.rs:93-109`), converted to
  `DfmMark` only when the anchor is finite (`:163-181`). No anchor ⇒ never a mark (`:164`).
* Projection: `Proj::to_screen` (`viewport.rs:386`); model screen box from the 8 AABB corners
  (`dfm_overlay.rs:333-349`).
* Label layout: two columns left/right of the model box, sorted by anchor Y, no overlap, 4-step
  push past obstacles, clamped to the view (`dfm_overlay.rs:361-412`); `LABEL_WIDTH=168`,
  `LABEL_HEIGHT=22` (`:23-24`).
* Obstacle: the Assistant panel rect (`assistant_obstacle`, `:355`; fed from
  `app.agent_chat.panel_rect`, `show:448`) — this is the *only* panel-aware part of the overlay, and
  it exists because the native view covers what egui paints.
* Hit test: plain rect containment over last frame's labels (`:415-417`), backed by
  `ViewportState.dfm_labels` (`viewport.rs:78`).
* Marks on the model: `build_highlight` (`viewport.rs:4270-4291`) tints each named face through
  `solidcraft_engine::doc::naming::locate_face(&st, name, mark.anchor)` (same path as refused
  faces, alpha 0.32) and draws `outline` as 2.6 pt polylines; the highlight cache key includes
  `dfm_marks.hash` (`viewport.rs:3874`).

### 6.4 The two directions, and the Assistant

* **Review row → viewport**: `dfm_review.rs:409` `if row(...)` → set `dfm_chat.selected`, call
  `viewport.select_dfm_finding(Some(&finding.id))`. Rows are grouped failed/advisory/checked/passed
  then any other state (`GROUPS`, `:29`), capped at 200 (`:26`).
* **Viewport label → Review row**: `dfm_overlay.rs:478-480` → `select_dfm_finding`; next frame the
  same `selected_key()` paints the row selected (`dfm_review.rs:385, 408`) and `:476` fills it.
* **Assistant card → viewport**: page posts `{"type":"finding","runId","id"}` (`App.tsx:419`) →
  `web.rs:601` → `dfm_chat::select` (run/id validation) → `viewport.select_dfm_finding`.
  Card data itself arrives as a tool row of kind `dfm.check` (`web.rs:258-271`), written by
  `run_check:893` / `poll_check:917` / `set_check_item:978`.
* **Viewport → Assistant**: every frame's state JSON carries
  `dfm:{display, selected:{runId,findingId}}` (`web.rs:333-336`, `selected_finding:970`), the page
  compares `item.selected.runId === card.runId && …findingId` (`App.tsx:410`) and shows
  `strings.dfmSelect` (`:428`).
* **Drag the report into the Assistant**: `dfm_review.rs:336-370` (grip, ghost), `:193-227`
  (drop hint + release), `:177-190` attachment, `agent_chat/attach.rs:134 dfm_report`,
  `:152 is_report`, `:157 report_run`. Sent text re-reads the named run, never "latest"
  (`agent_chat/mod.rs resolve_reports`, `:1553`).

### 6.5 Stale after a model change — what is hidden/refused

| Fact | Behaviour | Cite |
|---|---|---|
| not `Current` | `visible_marks()` empty, marks not drawn | `dfm_overlay.rs:265-270` |
| not `Current` | `select()` returns false and clears selection | `:285-297` |
| not `Current` | `locate_target()` none, `locate` cancelled | `:221-223, :228-233, :300-302` |
| review card | badged `… · stale` in warning ink | `dfm_review.rs:349-353` |
| page | `dfmStale` badge | `App.tsx:384` |
| re-run | old marks dropped, stale selection dropped | `dfm_overlay.rs:214-224` |
| new design | panel clears, reloads newest report | `dfm_review.rs:245-250` |

**Gap to price:** nothing re-reads freshness on an edit. `set_dfm_freshness` has **no production
caller** (only `viewport.rs:343` + tests), `dfm_review` keys on `document_id` only (`:245`),
not `revision`. So marks drawn before an edit stay drawn (and stay clickable) until the panel is
reopened, a check is polled, or the page polls. Only then does the report go `unknown`/stale and
every mark vanish.

### 6.6 WebView state JSON (exact shape)

```
host → page, once per frame (web.rs:~500-570):
  { …chat…, "strings": {…dfmCheckTitle, dfmStale, dfmAll, dfmSelected, dfmOff, dfmSelect…},
    "dfm": {"display":"all|selected|off",
            "selected":{"runId":…,"findingId":…}|null} }
  per item: {"kind":"check","id","status":"pending|running|done|error",
             "card":{runId,candidateDigest,configDigest,modelVersion,projectId,freshness,
                     findings:[{id,sheet,body,rules,state,status,code,measured,requirement,feature,
                                anchor,flat,faces,outline,reason}],
                     rules,notChecked,status,outcome,error,available,counts},
             "display","selected"}
page → host: {"type":"dfm_check"} | {"type":"dfm_poll"} |
             {"type":"finding","runId","id"} | {"type":"dfm_display","mode"}
```

---

## 7. Test / E2E seams

| Seam | Reachable in the product? | Evidence |
|---|---|---|
| `engine.execute {"command":"dfm.check"}` / `inspect.dfm` | **yes**, control channel | `docs/control-protocol.md:590`; `cmd/dfm_check.rs` |
| `project.data_set`, `project.create`, `project.ci_set` | yes | `cmd/project.rs:79,31,73` |
| `ui.prefs {"right_rail":true,"right_rail_open":"review","right_rail_width":…}` | yes | `right_rail_tests.rs:12,213` |
| `ui.handles` (points) → `review:run`, `review:card`, `review:finding:<id>`, `dfm_review:drop_hint` | yes, published each frame | `dfm_review.rs:308, 356, 499, 208`; `scenario::publish_target` `scenario.rs:76-82`; `control.rs:1137` |
| `ui.click/ui.drag/ui.move` at those points | yes | `control.rs:1022,1050` |
| `ui.screenshot` | yes (page renders black — it is a native child, see `docs/dfm-acceptance-report.md:162`) | `control.rs:1409` |
| viewport DFM label rects | **no** — `dfm_overlay::show` publishes no handle or rect | `dfm_overlay.rs:440-483` (no `publish_*`) |
| `rect_of()` for tests | test-only (`pub` but not exposed by control) | `scenario.rs:99` |
| `Harness` scenario runner (`ui.*` calls, click-by-handle, `settle`) | test-only crate API | `scenario.rs:697`, `dfm_review_tests.rs:9-20` |
| `dfm_run_findings.json` (run-7, 4 findings, 3 with anchors; `sheet-1/F2` and `sheet-1/F4` share `sheet:Bracket1:base:top`) | **test-only fixture**, `include_str!` | `dfm_overlay.rs:491`; `test_data/dfm_run_findings.json` |
| `dfm_overlay::tests` (9 tests incl. two-on-one-face, stale, rotation/zoom, panel avoidance) | test-only | `dfm_overlay.rs:485-667` |
| `viewport_navigation_cache_tests.rs:145-158` marks change the highlight key | test-only | — |
| `dfm_review_tests.rs` (5 tests: run entry, row→viewport, unsupported row, drop, drop elsewhere) | test-only, drives real product fns | `dfm_review_tests.rs` |
| `right_rail_tests.rs` (rail open/close, widths, Review entry `:76`) | test-only | — |
| `tests/right_rail_render.rs` (pictures of the Review rail) | integration test | `:69-89` |
| No DFM `.json` scenario, no `ui.dfm` control method | **gap** | `ls crates/ui-egui/tests/scenarios | grep dfm` → none; `control.rs` has no `dfm` dispatch |

### How to run the DFM E2E (route A, real desktop, two findings on one face)

Two findings on one face is *produced*, not injected: a 60×40 native sheet with **two Ø2 holes**
(`plate` sample) gives two `sm.hole_min_diameter` failures on the same face, plus a
`sm.hole_to_edge` one. (`spacing` with two Ø3 holes 4 mm apart gives the same shape for
`sm.hole_spacing`; the tray desktop run showed two `! Failed · sheet-1/F20` labels.)

```sh
# 1. build (one heavy job at a time; Nix store does not count against the 200 GiB budget)
nix build .#solidcraft            # or: CARGO_TARGET_DIR=… cargo build -p solidcraft --release
BIN=./result/bin/solidcraft

# 2. an isolated home + a project the app can write
export HOME=/tmp/opencode/int1454-v2/home XDG_*_HOME=$HOME/.local/state
$BIN --control 7411 &            # control channel is on by default; --control pins the port
CTL='nc -N 127.0.0.1 7411'       # or the repo's control client; one JSON object per line

# 3. A TEST-ONLY factory profile must be selected, or every numeric rule is `unsupported`
{"id":1,"method":"engine.execute","params":{"command":"project.create","params":{"name":"dfm-e2e","part":"Plate"}}}
{"id":2,"method":"engine.execute","params":{"command":"project.data_set","params":{"kind":"factory",
   "path":"crates/engine/src/project/test_data/critical_shop.json"}}}

# 4. the part, through the normal commands (same sequence as tests/dfm_acceptance.rs:82-87,168-172)
{"id":3,"method":"engine.batch","params":{"commands":[
  {"command":"sheet.manage_rules","params":{"name":"Acceptance","thickness":"2 mm","k_factor":"0.5",
     "bend_radius":"1 mm","relief_width":"4 mm","relief_depth":"4 mm","active":true}},
  {"command":"sketch.create","params":{"plane":"XY","name":"Base"}},
  {"command":"sketch.rectangle.two_point","params":{"p0":[0,0],"p1":[60,40]}},
  {"command":"sketch.finish","params":{}},
  {"command":"sheet.flange","params":{"sketch":"Base"}},
  {"command":"solid.hole","params":{"position":[30,20,2],"diameter":2}},
  {"command":"solid.hole","params":{"position":[2,20,2],"diameter":2}}]}}

# 5. run it and wait (the worker is a thread; ui is usable meanwhile)
{"id":4,"method":"ui.prefs","params":{"right_rail":true,"right_rail_open":"review"}}
{"id":5,"method":"engine.execute","params":{"command":"dfm.check","params":{}}}
{"id":6,"method":"engine.execute","params":{"command":"dfm.status","params":{}}}
{"id":7,"method":"engine.execute","params":{"command":"inspect.dfm","params":{}}}     # expect current

# 6. drive the UI by published handles, then look
{"id":8,"method":"ui.handles","params":{"prefix":"review:"}}    # review:card, review:finding:sheet-1/F*
{"id":9,"method":"ui.click","params":{"x":<cx>,"y":<cy>}}        # row → mark, check dfm labels on screen
{"id":10,"method":"ui.screenshot","params":{"path":"/tmp/…/dfm-rows.png"}}
{"id":11,"method":"ui.handles","params":{"prefix":"rail:"}}      # rail:panel to confirm the panel rect
```

Label→row cannot be driven this way today (no published label rect); it is covered only by
`dfm_overlay::tests` and `dfm_review_tests.rs`. Adding `publish_target("dfm:label:<id>", rect)` in
`dfm_overlay::show` is a ~3-line change that would close this E2E gap for both routes — recommended
before any route decision is tested on hardware.

Stale check, same session: change the model (`solid.hole`, another Ø2 at `[45,20,2]`), then
`inspect.dfm` → expect `"freshness":"unknown"`, `dfm_review` panel badge, and no marks.

---

## 8. Ownership table (A vs B)

| Concern | A owner (file:line) | B owner | Keep / adapt / new for A→B |
|---|---|---|---|
| Window, eframe loop | `apps/solidcraft/src/main.rs:195,1226` | same file, `:195,1224` | **keep** (B is nearly identical here) |
| Editor UI crate | `crates/ui-egui` (321 src files) | same (338) | keep, but every `ui_kit::` path changes owner |
| Kit | in-crate `ui_kit/` 66 files, `lib.rs:317` | `solidcraft-ui-kit` + `ui-text/theme/look/ui-registry/color` | **new** at B; adapters only if A is the future |
| Panel shell | `shell.rs`, `frame.rs`, `Container` edges | `panel_runtime/*` (12 files) + kit `Container` | A: keep; B: new shell, DFM panel must plug into `panel_runtime` |
| Camera input | `navigation.rs:548` | `navigation.rs:676` | **keep verbatim** |
| Pick / projection | `viewport.rs:2541/2585/379` | `viewport.rs` (15 115 lines) + `crates/viewport*` | keep; B's split is scene-side |
| Report data | engine `dfm/run.rs`, `findings.rs`, `service.rs`, `cmd/dfm_check.rs` (3 352 lines incl. tests) | **absent** (only `engine/src/dfm.rs`, 817 lines, library-level `check_sheet`) | **new at B** |
| Run identity/freshness | `dfm/run.rs:159`, `service.rs:322-377` | absent | **new at B** |
| Selection key | `ViewportState.dfm` `viewport.rs:76`, `dfm_overlay.rs:187` | absent | **new at B** (or port `dfm_overlay.rs` 667 lines) |
| Anchor/faces/outline | `dfm_overlay.rs:93-181`, `viewport.rs:4270` | absent | **new at B** |
| Projection of marks | `dfm_overlay.rs:333`, `viewport.rs:386` | `Proj` equivalent exists | **adapt** (same `Proj` type name both refs) |
| Label layout/hit test | `dfm_overlay.rs:361-417` | absent | **new at B** |
| Review panel | `dfm_review.rs` 512 + `right_rail.rs` 335 | no rail file at all | **new at B** (rail + panel + tests) |
| Assistant card bridge | `agent_chat/web.rs:258/333/601/893/917/970` | `web.rs` exists, no DFM actions | **adapt** (add 4 actions + 2 state keys) |
| Report attachment | `agent_chat/attach.rs:134-160` (+`dfm_runtime.rs` 287) | `attach.rs` exists (868 lines), no report kind | **adapt** (+~110 lines, `dfm_runtime.rs` new) |
| Native WebView host | `apps/solidcraft/src/assistant_web.rs` (1100) | same, **older** (no `latest`/GTK sync, no clipboard feature) | A: keep; B: **adopt A's INT-1472 fixes** |
| Panel/resize contention | `panel.rs:145-283`, `shell.rs:188` | `panel_runtime/interaction.rs` + `panel.rs` | A: keep; B: **re-derive** in the retained shell |
| Command schemas | `cmd/*.rs` `CommandSpec` + `schema/` | present (older set) | keep; DFM specs are **new at B** |

### Route A vs route B, separated

**Route A (keep IntCAD's UI, port upstream work into it)** — what it costs on this boundary:
keep `navigation.rs`, `viewport.rs` pick path, `ui_kit/`, `shell.rs`, `dfm_*`, `right_rail.rs`
as-is. Work is limited to: (1) the stale-on-edit gap (§6.5), (2) publishing label rects for E2E
(§7), (3) reconciling `dfm_chat.selected` with `viewport.dfm.selected` for cross-run ids (§6.2),
(4) whatever upstream features the lead chooses from the other agents' reports. No kit licence
surface, no re-implementation of 5 DFM files or the rail.

**Route B (#145's new shell)** — work *forced only by the new shell*, on top of the kit swap:
1. Port the DFM engine slice: `dfm/{run,findings,service,compile,export}.rs` + `cmd/dfm_check.rs`
   + `schema/dfm.rs` (3 352 lines with tests) onto B's engine, which has a different validation
   pipeline (`validation/runner.rs` exists at B, `dfm.rs` is a 817-line library shim).
2. Port `dfm_overlay.rs` (667), `dfm_chat.rs` (340), `dfm_review.rs` (512), `right_rail.rs` (335)
   and their tests (569 lines), against B's kit API. That API is **not** a drop-in for A's in-crate
   kit: B's callers use names A does not have (`ui_kit::LookStep`, `ThemeSegment`, `ThemeGlyph`,
   `PreferenceFigure`, `CheckStatus::{Ok,Warning,Error}` — `grep -rn "ui_kit::"` in
   `render_workspace_output.rs:1296`, `prefs/theme_picker.rs:3`, `prefs/graphics.rs:4`,
   `render_ui/appearance.rs:10`), and A's `ui_kit::text/Button/Container` are local. Expect an
   adapter pass over every DFM panel call, not a rename. There is **no** `right_rail` anywhere at B
   (`grep -rl right_rail src-b/crates src-b/apps` → 0 files), so the rail itself is new.
3. Re-add the 4 `web.rs` actions + 2 state keys + `attach::dfm_report`, and re-publish
   `review:*`/`rail:*` handles through whatever `scenario`/`publish_target` exists at B.
4. Re-plug the Review panel into `panel_runtime` (zones, tabs, `place_assistant`) rather than a
   `Container` edge, and re-test splitter/capture in the floating layout.
5. Adopt A's Assistant placement fixes into B's `assistant_web.rs`.

Note that (1)–(3) are *the same work* route A already has, so B pays it twice: once to get the
features, then again to keep them in sync with develop while #145 is open.

---

## 9. Residual risks and open questions

* **Native-view z-order**: `ui.screenshot` cannot capture the Assistant page
  (`docs/dfm-acceptance-report.md:162`), so any DFM card screenshot must be taken from the X root
  window. Budget for that in evidence.
* **`Unknown` freshness**: because no caller sends `modelDigest`, the "staleModel" badge is
  practically unreachable from the desktop UI; only a config change yields `staleConfig`. If the
  lead wants the §3 contract wording honoured, the UI must pass the live candidate digest.
* **No publication of label rects**, so the viewport→row direction has no control-protocol seam.
* **`panel_rect` is the only obstacle the overlay knows**; any future floating sheet over the
  viewport will have labels drawn under it.
* `right_rail.rs` says "Nothing fills the panels yet" (`:17`) but Review is filled — stale comment,
  harmless.

---

## 10. Reproduce this trace

```sh
cd /home/hyjing/Code/intgral/intcad
mkdir -p /tmp/opencode/int1454-v2/src-a /tmp/opencode/int1454-v2/src-b
git archive 02a786d47180ffa44a99f0508749c1c11b383c0d | tar -x -C /tmp/opencode/int1454-v2/src-a
git archive f45d977481614fad850c7f9ca67e48e6e7956e5f | tar -x -C /tmp/opencode/int1454-v2/src-b

# the two claims that drive the cost estimate
grep -rl ui_kit /tmp/opencode/int1454-v2/src-a/crates/ui-egui/src | wc -l      # 226
ls /tmp/opencode/int1454-v2/src-b/crates/ui-egui/src | grep -ci dfm             # 0
grep -rl right_rail /tmp/opencode/int1454-v2/src-b/crates /tmp/opencode/int1454-v2/src-b/apps | wc -l  # 0
ls /tmp/opencode/int1454-v2/src-b/crates/engine/src/dfm 2>&1                    # No such file
wc -l /tmp/opencode/int1454-v2/src-a/crates/engine/src/dfm/*.rs \
      /tmp/opencode/int1454-v2/src-a/crates/engine/src/cmd/dfm_check.rs \
      /tmp/opencode/int1454-v2/src-a/crates/engine/src/schema/dfm.rs            # 3352 total

# the DFM commits behind A
git show --stat 18366bbf0 183cf7836 4eb71a7a6
```

