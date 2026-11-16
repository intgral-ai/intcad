# INT-1454 · Upstream coverage: common ancestor → SolidCraft v0.41.0

Read-only investigation (research agent 2 of 3). No build, no tracked-file change, no push.
Every number comes from Git objects, so the ~1,900 untracked `*.sync-conflict-*` files in the
main checkout cannot affect any count.

## 0. Fixed baselines

| Ref | SHA | Note |
|---|---|---|
| origin/develop | `02a786d47180ffa44a99f0508749c1c11b383c0d` | current product |
| PR #145 base | `ac560eec548746872a6502f6df3a296cd08fea6d` | fork point, no brand crates |
| upstream v0.41.0 | `3c8efdb51952ed9a5f3ebc722c2bad1454fe6d80` | target |
| common ancestor | `06749ac464e16357d0375b78e6b3cac214066493` | = v0.40.0-rc.2 code, 2026-11-05 |
| brand licence first commit | `312454f899a371f6dac52348749bc4dcb94d94f0` | 2026-11-10 |
| pre-brand snapshot | `a34756830185f82da3a0e3ddd1ea25ef21111558` | `312454f89^` |

Commands used throughout:

```sh
git rev-list --count 06749ac..3c8efdb                       # 4009
git rev-list --count --first-parent 06749ac..3c8efdb        # 54
git diff --shortstat 06749ac 3c8efdb                        # 10338 files, +9494557 −72127
git diff --shortstat 06749ac 3c8efdb -- . ':(exclude)solidcore'
git diff --numstat -M 06749ac 3c8efdb                       # 559 renames
git rev-list --count 06749ac..3c8efdb -- <paths>            # per group
git cat-file -p 3c8efdb:LICENSE-BRAND
```

## 1. Shape of the net change (read this before the table)

The 4009 commits are **not** 4009 commits of work between 5 and 12 November. The range contains a
second root, `593a4c12f "Initial commit: gitignore"` (2026-10-12), which enters through one
subtree merge:

```
916b91e8d  Vendor solidcore at solidcore/ with its history (solidcore c0d3cc0124)
           git-subtree-dir: solidcore · git-subtree-split: c440e5f89
```

- solidcore lineage: **2800** commits (`git rev-list --count 916b91e8d^2`), rewritten to one
  commit per same-author run per PR, tree identical to solidcore main at `c0d3cc0124`.
- SolidCraft-side: **1209** commits (`git rev-list --count 06749ac..3c8efdb --not 916b91e8d^2`),
  of which 54 first-parent and 51 merges.
- Only **251** SolidCraft-side commits touch `solidcore/` paths
  (`git rev-list --count 06749ac..3c8efdb -- solidcore`); the other 2800 arrive in that one merge.

Size distribution (`git diff --numstat` aggregated by path):

| Path | files | lines | share |
|---|---:|---:|---:|
| `solidcore/fixtures` | 935 | 6,477,738 | 68% |
| `solidcore/crates` | 4,397 | 2,317,061 | 24% |
| `solidcore/plan` | 2,374 | 124,926 | 1.3% |
| everything else | 2,632 | 646,959 | 6.7% |

`solidcore` (kernel + fixtures + plan) is 92% of the diff. The open product surface is
**2,445 paths, +415,992 / −87,681** — that is the number that matters for a sync.

The *kernel* is therefore not a missing feature: IntCAD develop already consumes it as a pinned
git dependency.

```
# origin/develop:Cargo.toml:41-46
solidcore        = { git = "https://github.com/tangency-labs/solidcore", rev = "8ddce901671f..." }
solidcore-record = { git = "https://github.com/tangency-labs/solidcore", rev = "f5e8e9a29e0c..." }
```

v0.41.0 instead vendors it at `solidcore/` with path deps. `8ddce901` is not in this clone
(`git cat-file -t 8ddce901` → fatal), so the exact develop→v0.41.0 kernel delta is **insufficient
evidence**; it needs the tangency-labs/solidcore repo or network access. A bound: 4 days after
vendoring, the vendored tree already moved a lot —
`c440e5f89` (7565 files) → `3c8efdb:solidcore` (7924 files): **1049 changed, 410 added, 51 removed**.
A weekly vendored sync is therefore expensive; a rev bump is not.

## 2. Coverage table by group

Line/file counts use `-M` rename expansion (renamed file counted once, at its new path).
Commit counts are path-filtered and overlap between groups.

| Group | Commits (full / v0.40.1→v0.41.0) | Files | Lines | Representative commits | Develop status |
|---|---:|---:|---:|---|---|
| kernel/geometry (`solidcore/crates/*` except step/convert, `solidcore/harness`, `crates/{kernel,geom,smlib,dynamics,sketch,derive}`) | 330 + 2800 vendored / 74 | 3,743 | 1,384,697 | `87903ee6e` blend curve G0/G1/G2; `2ff59f798` solidcore-ops trim/cut/remove faces; `73459b282` sweep sharp corner; `548fc3004` nearest/farthest point; `9f114ce40` tangent faces join; `93c8b738a` 3012 sliver grading; `cd9fa5211` typed blend failure record | **Already, by dependency.** New kernel work = move the `solidcore`/`solidcore-record` revs. Doing it in-tree needs the vendoring decision (`solidcore-ops`/`-tess`/`-record` seam rule). Delta to the pinned rev unmeasured. |
| doc-format (`crates/doc`, `crates/format`, format `v51`–`v55`) | 165 / 38 | 158 | 23,459 | `80cab6930`; `b34c23d0c` drafted-face ellipse; `17d47326e` chamfer wider than its flat; `0e131c6cf` sheet-metal fold keeps faces; 46 format changes in `v51`–`v55` | **Adapt + conflict.** develop's last folded dir is `v50` with 5 pending files (writes 51); v0.41.0 is folded to `v55` with an empty `pending/`. Those 5 pending files are **byte-identical** to five files inside upstream `v51/` (§7). The meaning of 51 differs; reconcile before any doc cherry-pick. |
| engine/commands/schema (`crates/engine`, `schema/`, `apps/solidcraft-cli`) | 360 / 51 | 423 | 114,571 | `e6890a8bd` sketch projection onto corner names; `114bdff4e` GLB assembly nodes; `70a4fea07` extrude auto-op by exact point; `2167af4d6` install plugin from `.zip`; `9fc8caf83` reopening STEP asks nothing; `a2dd5a3ba` save progress + Cancel | **Direct absorb, per command**, where the command does not need a new UI surface. 360 commits is the largest open surface. |
| I/O (`crates/io`, `crates/step`, `solidcore-{step,convert}`, `step-export`, `step-import`) | 63 / 15 | 73 | 27,235 | `5980a21b5` AP242 product layer into solidcore-step; `3453963cf` STEP export inside-out; `eee6c7ae5` cone/sphere seam faces; `566c1027f` "largest file to open" honoured; `114bdff4e` GLB nodes | **Mixed.** Develop already has `crates/kernel/src/step_out.rs` (AP242 product structure) — upstream moved that layer into `solidcore-step`, so `5980a21b5` is an **architecture mismatch (adapt/decline)**, not a patch. GLB *export* is absent in develop (`crates/io/src` has `gltf_in.rs` only) → direct absorb. |
| viewport/render/GPU/control (`crates/{render,gpu,viewport*,control*,color,space-mouse}`) | 38 / 7 | 203 | 64,264 | `5c99792a8` viewport split into 3 crates; `295a5e141` shared control client; `08ad6f10c` path-tracer shaders 21.6 min → seconds; `acb532e16` instanced preview copies; `1bacd8323` perspective zoom far from origin | **Adapt.** Develop has `gpu`, `render`, `space-mouse` but no `viewport*`, `control*`, `color`. Individual fixes inside `crates/gpu`/`crates/render` are direct; the crate split is an IntCAD interface decision. |
| plugins (`crates/{craft-plugin,craft-plugin-wasm}`) | 1 / 1 | 13 | 2,147 | `53a76b23f` sandboxed wasm host (permissions, no network); `2167af4d6` install from zip; `ae346a7b2` Plugins menu | **Adapt, cleanly open.** Develop already ships phase 0 — `crates/ui-egui/src/plugins.rs` (550 lines): "It does not load or run anything… the wasm host is a later step". `craft-plugin` is a dependency-free contract crate; `craft-plugin-wasm` pulls **73 new Cargo.lock packages** in the window, incl. `wasmtime` 48 + cranelift + `cap-std`, target-gated to x86_64/aarch64. |
| UI open (`crates/ui-egui`, `ui-text`, `ui-registry`, `apps/*`) | 343 / 53 | 439 | 143,376 | `a30b34707` one docking system; `8d47fc9f2` trackpad twist; `c1e1f3398` Blender input mode; `0d7a0c621` sketch snap points; `ce8e80c8f` operations batch rows | **Conflict.** Develop is 14+ UI commits ahead of the fork and keeps its own `src/ui_kit/` (66 files); upstream moves every component call site into `solidcraft-ui-kit`. Merge conflicts are structural, not textual. |
| UI restricted (`ui-kit`, `look`, `theme`, `assets/branding`, `assets/icons/app`) | 27 / 10 | 556 | 30,021 | `b34a0ad74` ui-kit crate created (MIT); `312454f89` brand licence; `7e902aa47` kit carries no SolidCraft content; `419bbcda0` theme crate; `7081f99b1` toast; `865497c70` isoparm colour | **Restricted.** 100% of these 30,021 lines. Pre-brand snapshots exist for most of it (§3). |
| Assistant/MCP/WebView (`crates/{mcp,mcp-server,agent-task*,threedx-client}`) | 20 / 3 | 16 | 4,009 | `295a5e141` control client + MCP bridge; `18928334c` `control.cancel` + MCP cancel; `26a643a7b`/`a17e1d2c9` compact replies; `c2ec24f0a` server prompts/resources/templates | **Direct absorb** for the MCP-side diffs (develop already has compact replies — 515 `compact` hits). `control` lives inside `ui-egui` in develop (`src/control.rs`, `control_job.rs`), so the shared-client commit is an interface move. |
| build/packaging/CI/release (`.github`, `packaging/`, `xtask/`, `scripts/`, `tools/`, `vendor/`, root manifests) | 146 / 38 | 166 | 21,070 | `e0180ff4a` release builds back on GitHub runners; `1c592927b` CI reuses the CLI; `916b91e8d` vendoring; `xtask/mesh-decisions.txt`; 4 new solidcore workflows | **Adapt, per file.** Nothing here is brand material. `solidcore-mirror.yml`, `solidcore-{nightly,pages,release}.yml`, `kernel-rig.yml` are kernel-repo plumbing IntCAD does not need as-is. |
| tests/corpus (`tests/`, `examples/`, `crates/testkit`, `solidcore/fixtures`, `solidcore/crates/goldens`, `bench`) | 133 / 26 | 1,824 | 7,473,853 | fixtures from upstream `TestFiles/pt_TestFiles/`; `93c8b738a` sliver grading | **Direct absorb** (largest single payload; already covered by solidcore's NOTICE/ATTRIBUTION, Apache-2.0). Check the 6.5M-line fixture blob against IntCAD's 200 GiB budget before pulling it in. |
| docs/planning (`docs/`, `plan/`, `skills/`, `solidcore/{plan,docs,reports,site}`) | 2,142 / 59 | 2,667 | 231,493 | `17c161bd5` rebase/merge docs | **Mostly skip.** 2,374 of these files are `solidcore/plan` (the port's own planning docs); `docs/` proper is 180 files / 13,844 lines. |
| assets (fonts, materials, samples, shaders, platforms) | 212 / 22 | 56 | 45,513 | `14a9cf73f` Korean Hangul face (type-face table is in `ui-kit/src/type_face.rs`) | **Split.** `assets/icons/ui`, `assets/styles/ui`, `assets/fonts` are still present in develop and identical to pre-brand (§3). |

Totals: 10,338 paths, +9,494,557 / −72,127.

## 3. The restricted boundary (`LICENSE-BRAND`)

Read verbatim (`git show 3c8efdb:LICENSE-BRAND`). §1 covers exactly: `crates/ui-kit`
(`solidcraft-ui-kit`), `crates/look` (`solidcraft-look`), `crates/theme` (`solidcraft-theme`),
`assets/branding`, `assets/icons/app`. §1 also carves out third-party material, naming the IBM Plex
fonts as OFL 1.1 and saying "nothing here limits what that licence allows". §3a forbids "linking to
the UI kit, the look or the theme from software that is not SolidCraft" without written permission
— that is the clause an IntCAD route has to respect, and it is a *linking* clause, not only a
copying clause. §5: "Versions of SolidCraft published before this licence took effect stay under
the licences they were published with."

### 3.1 What moved across the boundary (496 renames)

`git diff --numstat -M` reports 559 renames; **496** cross the open/restricted line, all one way:

| Direction | Count | Example |
|---|---:|---|
| `assets/icons/ui` → `crates/look/icons` | 398 | `assets/icons/ui/design/svg/flat/assistant.svg` → `crates/look/icons/design/svg/flat/assistant.svg` |
| `crates/ui-egui/src/ui_kit` → `crates/ui-kit/src` | 69 | `crates/ui-egui/src/ui_kit/actions.rs` → `crates/ui-kit/src/actions.rs` |
| `assets/styles/ui/themes` → `crates/look/themes` | 24 | `assets/styles/ui/themes/nord.json` → `crates/look/themes/nord.json` |
| `assets/styles/ui/palettes.rs` → `crates/theme/src/palettes.rs` | 1 | |
| `assets/fonts` → `crates/look/fonts` | 4 | IBM Plex (OFL, stays OFL under any licence) |

No renames go restricted → open.

### 3.2 IntCAD already owns most of the moved material

Comparing the pre-brand snapshot and IntCAD develop against the restricted destination:

```sh
# blob-hash comparison, pre-brand a3475683 / develop / v0.41.0
assets/icons/ui            pre-brand 403 · develop 400 · identical 393 · v0.41.0 401 (390 identical to pre-brand, 2 added since, 9 changed, 4 dropped)
assets/styles/ui/themes    pre-brand  25 · develop  25 · identical  24 · v0.41.0  25 (all 25 content-changed since the move)
assets/fonts               pre-brand  10 · develop  12 · identical  10 · v0.41.0   4 (OFL, unaffected by the brand licence)
```

So the icon masters, the theme JSONs and the fonts are **already in IntCAD's tree under
MIT/Apache**, inherited at the fork and confirmed by byte-identical blobs. What the brand crates
add is *code*, not artwork: `crates/ui-kit/src` (105 files, 2.38 MB), `crates/theme/src`
(96,672 B `lib.rs` + 28,947 B `palettes.rs`), `crates/look/src` (8 files, 38,641 B:
`icons.rs`, `glyphs.rs`, `compass.rs`, `fonts.rs`, `pointer.rs`, `themes.rs`, `lib.rs` + tests).

The restricted code has direct functional counterparts already in develop's open tree:
`crates/ui-egui/src/{theme.rs,theme_files.rs,theme_options.rs,icons.rs,icon_svg.rs,scene_palette.rs,sketch_palette.rs}`
plus `assets/icons/ui/source/{glyphs,markers,navigation,pointer}.rs`.

## 4. Mixed commits (open + restricted in one commit)

25 of 4009 (9 of 203 in the v0.40.1→v0.41.0 window). Script: walk `git log --name-only`, split each
commit's paths on the `LICENSE-BRAND` prefixes.

| SHA | Restricted / open files | Subject | What is genuinely restricted |
|---|---:|---|---|
| `a30b34707` | 9 / 140 | Use one docking system in Design and Render | 9 `ui-kit` docking files; the 140 open files are engine/doc/viewport wiring |
| `53a76b23f` | 2 / 68 | Sandboxed WebAssembly plugin host, phase 1 | `ui-kit/src/prefs_window.rs` + test: the Plugins preferences tab |
| `2b936bd7b` | 2 / 6 | File export: save bodies as a GLB file | `ui-kit/src/app_bar.rs` + test: one menu item |
| `7e0e63504` | 1 / 47 | Spline tangent handles at every fit point | `ui-kit/src/menu.rs` (+88): the context-menu entry |
| `487a65d0a` | 2 / 5 | Large assemblies: a third less work per frame | `ui-kit/src/icons/svg.rs`: icon cache |
| `c1500382e` | 1 / 8 | Sketch dimension text size and colour | `ui-kit/src/preferences_small.rs` (+4) |
| `ce8e80c8f` | 1 / 6 | Operations panel batch rows | one `ui-kit` row widget |
| `14a9cf73f` | 1 / 9 | web: Korean UI loads its own Hangul face | `ui-kit/src/type_face.rs` (+6) |
| `7cc64dbb3` | 2 / 17 | Assemblies: Constrain dialog | 2 `ui-kit` dialog files |
| `95dec905c` | 2 / 16 | Construct Plane/Axis/Point by what you pick | `crates/look/icons/.../plane_auto.svg` + `requested_icons.txt` — **an icon, and the icon is already ours pre-brand** |
| `ae346a7b2` | 2 / 6 | A Plugins menu | 2 `ui-kit` menu files |
| `7081f99b1` | 5 / 3 | View toast is a kit component | the component itself (brand-only, post-relicence) |
| `865497c70` | 28 / 1 | theme: isoparm colour | all 28 (palette/token data) |
| `b55c3c2da` | 4 / 20 | Tooltips: eleven callers converted | tooltip container in the kit |
| `7e902aa47` | 2 / 80 | UI kit: no SolidCraft content; host data | small; the open 80 files are the host-data plumbing |
| `b34a0ad74` | 102 / 42 | UI kit: the shared crate (closes #14251) | the 102 are pure renames out of `ui-egui/src/ui_kit/` |
| `419bbcda0` | 5 / 252 | Shared token contract `solidcraft-theme` | `theme/src/lib.rs` (+1891), `palettes.rs` move, `ui-egui/build.rs` −16 |
| `312454f89` | 451 / 57 | Look: icons, glyphs, fonts, themes as `solidcraft-look` | **the licence boundary itself**: moves 451 files, adds `LICENSE-BRAND`, rewrites `ATTRIBUTION.md` (905 lines), `NOTICE` |
| `2874e782f` / `46bb71b5e` / `017c47a40` / `4eaa0adb8` / `5db0502eb` / `0e6927672` / `bd69229bf` | 1–5 / 3–26 | kit works under another host's name; host icons in menus; agent-chat mentions; sketch text vertical align; dialog first-letter buttons; assembly origin datums; inline browser rename | 7 commits, 21 restricted files in total, each a small kit API change beside a real open feature |

Pattern: **the restricted half is almost always the widget, the menu item, the preference row or the
icon that exposes the feature**; the feature itself (doc/engine/io/kernel) is open and separable. The
three genuine exceptions are the boundary-creating commits (`b34a0ad74`, `419bbcda0`, `312454f89`)
and the post-relicence components (`toast.rs`, `confirm_tests.rs`, `865497c70` tokens).

For `312454f89` the open side is 57 files and is *not* feature work: `LICENSE-BRAND` (new),
`NOTICE` (+9), `ATTRIBUTION.md` (rewritten), `README.md`, `CONTRIBUTING.md`, `Cargo.toml`/`Cargo.lock`,
`.gitattributes`, `apps/*/Cargo.toml` + `main.rs` (+2/+3: install `solidcraft-look`), `assets/AGENTS.md`.

## 5. Does open code depend on the restricted crates?

Yes, but only at the UI shell — and that is the whole escape.

`cargo metadata --no-deps` at v0.41.0 (manifests extracted with `git archive` into a throwaway tree;
`src` targets stubbed so the workspace parses; `--offline`; exit 0):

```sh
git ls-tree -r --name-only 3c8efdb | grep -E '(^|/)Cargo\.toml$|(^|/)build\.rs$|^\.cargo/' | grep -v '^solidcore/'
git archive 3c8efdb <those paths> | tar -x -C /tmp/opencode/int1454-v2/v041
cd /tmp/opencode/int1454-v2/v041 && cargo metadata --no-deps --format-version 1 --offline
```

Dependency edges onto the restricted set (36 workspace members):

| Package | Depends on |
|---|---|
| `solidcraft` (desktop app) | `look`, `theme` |
| `solidcraft-web` (wasm app) | `look` (and `ui-egui`, hence `ui-kit`) |
| `solidcraft-ui-egui` | `ui-kit` (normal + `dev`/`test-support`), `look`, `theme` |
| `solidcraft-look` | `ui-kit`, `theme` |
| `solidcraft-ui-kit` | `theme` (plus open `color`, `ui-registry`, `ui-text`, `viewport-egui`, `viewport-gpu`) |

**Not** dependent: `engine`, `doc`, `io`, `step`, `kernel`, `smlib`, `geom`, `sketch`, `dynamics`,
`gpu`, `render`, `craft-plugin`, `craft-plugin-wasm`, `mcp`, `mcp-server`, `space-mouse`, `profile`,
`testkit`, `agent-task*`, `threedx-client`, `solidcraft-cli`.

So the direction is restricted → open (the kit consumes open widgets), plus one shell edge
(`ui-egui` → kit). Nothing in the geometry/IO/engine/plugin/MCP stack touches brand material.

Other embedding surfaces checked with `git grep -n 'include_bytes!\|include_str!'` over `crates/*`
and `apps/*` at v0.41.0: **no open crate embeds a file from a restricted path.** The nearest hits
are textures (`assets/render/starter/...`) and sample PNGs (`assets/samples/...`), both open. The
only build-time cross-boundary read is `apps/solidcraft/build.rs:4` →
`assets/icons/app/windows/exports/solidcraft-executable.ico` (the app crate, brand icon).
`crates/look/build.rs` compiles its icon masters and theme files into `$OUT_DIR` tables — "Every
path is inside this crate" — so a `look`-free build cannot regenerate them by pointing elsewhere.
`crates/ui-egui/build.rs` reads `assets/locales` (open) only.

### 5.1 What the artefacts actually carry

- **Installed packages** (`packaging/linux/package.sh`, `packaging/env.sh`): three binaries
  (`solidcraft`, `solidcraft-cli`, `solvecraft-cli` shim), desktop/mime/metainfo files, app icons
  copied from `assets/icons/app/...` (restricted), and `README.md`, `LICENSE-MIT`,
  `LICENSE-APACHE`, `NOTICE`, `THIRD-PARTY-LICENSES.txt`. `copy_docs()` does **not** copy
  `LICENSE-BRAND`. The binary itself carries the compiled look/theme tables and ui-kit — i.e. the
  redistributable artefact *is* brand material plus, in this packaging, without its licence text.
- **Web** (`packaging/web/package.sh`, `pages.yml`): the wasm bundle at `/app/` includes
  `look`→`ui-kit`→`theme` compiled in.
- **Source tarball**: there is none. `release.yml` ships binary `.tar.gz` only (asserted:
  `tar -tzf dist/release/*.tar.gz | grep -E '/bin/solidcraft$|THIRD-PARTY-LICENSES.txt'`). The only
  `git archive` in CI is `ci.yml:1306` (`git archive HEAD solidcore`, for the mirror job). GitHub's
  auto-generated source archive, and `flake.nix` builds from the repo, would of course include the
  restricted tree.

## 6. Historical sync replay: v0.40.1 → v0.41.0

Window choice: v0.40.1 (`0c5292285`, 2026-11-11) is the last stable before the target and v0.41.0 is
the next day, so this is a one-day window, not a week — **203 commits, 892 files, +67,333 / −16,111**.
It is still the right window: it is *after* the boundary was drawn (2026-11-10), so it tests steady
state. The boundary-crossing week is the 06749ac→v0.41.0 range in §2–§4. v0.40.0→v0.41.0 would add
only the 68 same-day patch commits (`d806707b8`→`0c5292285`) and no new structure.

Per group in the window (`git rev-list --count v0.40.1..v0.41.0 -- <paths>`):

| Group | Commits | Notable |
|---|---:|---|
| kernel/geometry | 74 | 11 of the 14 `mesh-decisions.txt` commits land here |
| engine/commands/schema | 51 | |
| doc-format | 38 | |
| ui-open | 53 | |
| build/ci/release | 38 | |
| docs | 59 | |
| tests/corpus | 26 | |
| assets | 22 | |
| io | 15 | |
| ui-restricted | 10 | 9 of them are the mixed commits below |
| viewport | 7 | |
| assistant/mcp | 3 | |
| plugins | 1 | `53a76b23f`, +73 Cargo.lock packages |

Simulating the proposed weekly process:

1. **Identify commits.** 203 by SHA; 9 touch the restricted boundary. A path-filtered queue
   ("only open paths") drops the restricted half of all 9 and silently loses the menu/prefs/icon
   that makes the feature reachable.
2. **File moves.** Zero boundary renames in this window (they all happened on 11-10). Across the
   full range there are 496, so a first sync *must* handle them; a later weekly sync never sees one.
3. **New restricted dependencies.** None added in the window (`git diff v0.40.1 v0.41.0 -- '*/Cargo.toml'`
   shows only `craft-plugin`/`craft-plugin-wasm`, both open). The edges were created earlier:
   `ui-egui → ui-kit` in `b34a0ad74`, `look → ui-kit, theme` in `312454f89`. A weekly checker that
   only diffs manifests would have missed both, because both were added by a *move* commit.
4. **New dependencies of any kind.** 73 lock packages in the window, essentially the wasmtime stack
   (`wasmtime`, `cranelift-*`, `cap-std`, `cap-primitives`, `ambient-authority`, `cobs`, `arbitrary`,
   `addr2line`, `cpp_demangle`). One plugin commit is a much bigger dependency event than its 2,147
   changed lines suggest.

What a rebase/cherry-pick tool would miss, concretely:

- **Renamed-into-restricted paths.** `git cherry-pick 95dec905c` (Construct Plane/Axis/Point) wants
  `crates/look/icons/design/svg/model/plane_auto.svg`; IntCAD has no `crates/look`. `-M` rename
  detection cannot invent the destination, and the open half then references an icon id that does
  not exist. Same for the 398 icon renames and the 69 `ui_kit/*.rs` renames.
- **Module → crate indirection.** Upstream edits `crates/ui-kit/src/menu.rs`; IntCAD's copy is
  `crates/ui-egui/src/ui_kit/menu.rs` reached through `pub mod ui_kit`. A path-based pick never
  matches; a semantic tool would still have to decide whether the kit API change belongs in the
  internal module at all.
- **Format-pending collapse.** IntCAD's `crates/doc/src/format/pending/` has 5 files that upstream
  folded into `v51/`, byte-identical (§7). Any doc cherry-pick re-adds files that already exist
  there under a different name, and `scripts/fold-format-pending.sh` at release would renumber
  them differently.
- **Generated files.** `docs/features.md` and `THIRD-PARTY-LICENSES.txt` change in this window
  (`git diff --name-only v0.40.1 v0.41.0 | grep -E 'docs/features.md|THIRD-PARTY'`); both repos
  forbid editing them by hand, so a pick lands a stale generated file that `cargo xtask lists`
  immediately fails on.
- **Pending locale/attribution files.** 14 `assets/locales/pending/*.json` and 5
  `attribution/pending/*.md` files in the window. They carry fine, but the fold step and the
  sorted-list check are separate and easy to forget.
- **`xtask/mesh-decisions.txt`.** 14 of 203 commits edit it, each deleting one
  `<path>:<function> …` line. IntCAD's list has extra lines and different content, so every one of
  those picks is a mechanical conflict *and* the allowlist gate has to be re-run.

## 7. Document-format collision (exact evidence)

```
origin/develop   last folded dir v50, pending/ = 5 files  → FORMATS.current() = 51
3c8efdb          last folded dir v55, pending/ = README   → FORMATS.current() = 55
v51 15 files · v52 20 · v53 3 · v54 1 · v55 7        (46 format changes, git ls-tree per dir)
```

`Formats::current()` (`crates/doc/src/format.rs:369`) is
`hand + 1 + folded + (1 if pending)`, so develop writes **51** while 5 changes are pending, and
upstream writes **55** with none.

The 5 pending files in IntCAD develop are **byte-identical** to 5 of the 15 in upstream `v51`:

| file | develop blob | v0.41.0 `v51/` blob |
|---|---|---|
| `bridged-counterbore.rs` | `5f960278` | `5f960278` |
| `fillet-turn.rs` | `34c49ca9` | `34c49ca9` |
| `hole-direction-own-faces.rs` | `481269c2` | `481269c2` |
| `revolve-axis-origin-expressions.rs` | `2b6ed033` | `2b6ed033` |
| `transferred-profile-frame.rs` | `0058834c` | `0058834c` |

IntCAD's 51 is not upstream's 51: upstream's `v51` holds those 5 changes **plus 10 others**. The
reader matches changes by name (`Formats::knows_part_name`, `parts()`), not only by number, so a
develop-written design claiming format 51 with 5 pending names will be read upstream as a v51 that
already has the other 10 — a data-interpretation conflict, not just a numbering one. (Inference
from `parts()`/`knows_part_name`; reading a real develop file with the v0.41.0 build would settle
it, and that needs a build.) This needs an
explicit IntCAD decision (most likely: drop the 5 pending files, adopt upstream `v51`–`v55`,
renumber any local change as `v56`) before any doc work is absorbed. Do not let a merge tool
"resolve" it.

## 8. Reconciliation with the earlier study

`/tmp/opencode/int1454-uikit/{reuse-map.md,call-audit.md}` and this inventory agree on the decisive
points, and I have not copied them blindly:

Agreed and independently re-verified here: `crates/ui-kit` was created **2026-11-10** in
`b34a0ad74` while the workspace was still `MIT OR Apache-2.0`; the brand licence arrives in the
**next** commit `312454f89` the same day; `a3475683` is a complete MIT/Apache snapshot of that crate
(77/78 non-test files; only `toast.rs` is brand-only); `crates/look` and `crates/theme` are listed in
§1 of `LICENSE-BRAND`; `ui-text`/`ui-registry` are not listed anywhere in §1.

Two corrections/additions from this pass:

1. `reuse-map.md` records for `crates/look`: "Brand licence from birth; no pre-Brand snapshot
   exists". True of the **crate**, false of its **payload**: 393 of 400 icon masters and all 25 theme
   JSONs existed pre-brand under `assets/icons/ui` and `assets/styles/ui/themes` and are identical
   in IntCAD develop today (§3.2). Only `crates/look/src` (38.6 KB of code) and `theme/src` are
   genuinely new-and-restricted.
2. Neither earlier file measures the dependency escape. The §5 metadata table is new evidence:
   `engine`, `doc`, `io`, `kernel`, `gpu`, `render`, `mcp`, `craft-plugin*` have **no** edge to the
   restricted crates; only `ui-egui`, `solidcraft`, `solidcraft-web` (and `look` internally) do.

## 9. Ranked open improvements worth absorbing (route A)

Ranked by user-visible value × absorbability without brand crates. "Brand tail" = restricted files
in that commit that must be re-implemented or dropped.

| # | Improvement | Commit(s) | Crates / files | Brand tail |
|---|---|---|---|---|
| 1 | **Mesh decision → exact geometry.** The strongest match to this repo's own rule: 18 commits in range (14 in the final week) each delete their own `xtask/mesh-decisions.txt` line, and **21 allowlist lines still present in develop** are gone at v0.41.0 — e.g. `eval.rs:body_at`, `hole_axis_size`, `features.rs:auto_operation_in`, `through_all_reach`, `history_names.rs:OperandNames::edge`, `joints_edge.rs:face_of`, `joints_face.rs:outline_centre`, `config_styles.rs`, `print_face.rs`, `document.rs:MeshData::eq` | `c17bd55fc` `5c98090ff` `70a4fea07` `51268523e` `dfdec70ee` `d18e1c6b5` `297bc8264` `2495988e3` `6a0849601` `908cfd07c` `80381ca4b` `b727fad01` `30d533df6` `027182bd8` | `crates/doc/src/{eval,history_names,joints_edge,joints_face,joints,config_styles,document,placement_reuse,print_face}.rs`, `crates/engine/src/cmd/{features,joints,motion,component_properties}.rs`, `xtask/mesh-decisions.txt` | **None.** Highest value/risk ratio by far. |
| 2 | **GLB export** (bodies now; assembly components and occurrences as named nodes) | `2b936bd7b` `114bdff4e` | `crates/io/src/glb_out.rs` (new), `crates/engine/src/cmd/export*`, `crates/doc` | 2 lines in `ui-kit/src/app_bar.rs` (one menu item) |
| 3 | **STEP robustness**: honour a raised "Largest file to open"; cone/sphere seam faces no longer dropped; reopening a STEP asks nothing; not inside-out | `566c1027f` `eee6c7ae5` `9fc8caf83` `3453963cf` | `crates/io`, `crates/kernel/src/step_in/*`, `step_out.rs` | **None** |
| 4 | **Kernel primitives and ops** (take by rev bump): blend curve G0/G1/G2, `solidcore-ops` trim/cut-along-curves/remove faces, sweep with a sharp corner, tangent booleans, nearest/farthest point | `87903ee6e` `2ff59f798` `73459b282` `9f114ce40` `548fc3004` | `solidcore/crates/{solidcore,solidcore-ops}`, `crates/kernel`, `crates/smlib` | **None** |
| 5 | **Mesh export path**: every face by constrained Delaunay with equilateral seeds (beside the display mesher) | `81dd685a8` | `solidcore/crates/solidcore-tess`, `crates/kernel` | **None** |
| 6 | **GPU/render**: path-tracer shaders compile in seconds instead of 21.6 min / 61 GB; instanced preview copies; perspective view when zoomed far from the origin | `08ad6f10c` `acb532e16` `1bacd8323` | `crates/gpu`, `crates/render`, `crates/viewport*` | **None** (viewport crate split is an interface choice, not brand) |
| 7 | **Plugin host**: sandboxed WebAssemblyComponent runtime with permissions and no network; install from `.zip`; Plugins menu | `53a76b23f` `2167af4d6` `ae346a7b2` | `crates/craft-plugin`, `crates/craft-plugin-wasm` (new), `crates/ui-egui/src/plugins.rs` (phase 0 already there) | 2 files: Plugins prefs tab + menu. Also **73 lock packages** and wasmtime native-only (needs a feature gate for the wasm app). |
| 8 | **Sheet metal**: flared wall folds with its rim; fold across several tabs keeps rounded corners as single faces; one flange splits into stretches | `5dabe7e73` `0e131c6cf` `13f2c2aa7` (23 sheet-metal commits in range) | `crates/doc/src/*sheet*`, `crates/engine`, `skills/solidcraft-sheetmetal` | **None** |
| 9 | **Sketch**: snap points; a point at a curve crossing anchors to both curves; spline tangent handles at every fit point; dimension text size/colour | `0d7a0c621` `53941c82e` `7e0e63504` `c1500382e` | `crates/sketch`, `crates/doc`, `crates/engine` | `7e0e63504`: `ui-kit/src/menu.rs` (+88). `c1500382e`: 4 lines in a prefs row. |
| 10 | **MCP/assistant**: `control.cancel` + MCP cancel tool; compact replies on large designs; server prompts/resources/templates; a `running` answer while a batch runs | `18928334c` `26a643a7b` `a17e1d2c9` `c2ec24f0a` `2ea022be8` | `crates/mcp`, `crates/mcp-server`; `control` is inside `ui-egui` in develop | **None**, but the shared `control` crate is an interface move |
| 11 | **Input**: trackpad two-finger twist; Blender input mode | `8d47fc9f2` `c1e1f3398` | `crates/ui-egui`, `crates/viewport*` | **None verified** in these two |

Explicitly flagged as restricted or new-UI-dependent, so *not* on this list: `a30b34707` "one docking
system" (9 restricted files; the docking lives in the kit), `7081f99b1` view toast (brand-new
component, post-relicence), `865497c70` isoparm colour (28 restricted token files), `2874e782f` /
`46bb71b5e` "kit works under another host's name" / "host icons in menus" (both are kit API changes
and §3a territory), `419bbcda0`/`b34a0ad74`/`312454f89` (the boundary itself).

## 10. Conclusion — what can be absorbed without the brand crates

**Almost everything that is geometry, I/O, engine, plugins or MCP.** The restricted set is
`crates/{ui-kit,look,theme}` + `assets/{branding,icons/app}` (30,021 changed lines — 6.0% of the
503,673 changed lines outside `solidcore/`). No open crate except `ui-egui` and the two app
shells links to it, and no open crate embeds a file from it.

Three separate things are commonly conflated:

1. **Brand artwork — already ours.** Icon masters (393/400), theme JSONs (24/25) and fonts (10/10)
   are in IntCAD develop, byte-identical to the pre-brand snapshot `a3475683`, under MIT/Apache by
   `LICENSE-BRAND` §5. No licence question arises for them. Keep using develop's copies; do not
   re-take them from v0.41.0, where 9 icons and all 25 theme files have changed.
2. **Brand code — not ours and not needed.** `ui-kit` (2.38 MB), `theme/src` (116 KB), `look/src`
   (38.6 KB). Develop has open counterparts for the parts that matter (`ui-egui/src/{theme*,icons,
   icon_svg,*_palette}.rs`, `assets/icons/ui/source/*.rs`). Every one of the 25 mixed commits in §4
   can be split: open half absorbed, restricted half re-implemented in IntCAD's internal module or
   dropped as chrome.
3. **Kernel — a dependency, not a merge.** Take kernel progress by moving the pinned
   `solidcore`/`solidcore-record` revs; the 2,800-commit vendored history and 6.5M-line fixture blob
   need not cross the boundary at all. If IntCAD does want to add kernel cases in-tree (as
   `AGENTS.md` implies), that is the vendoring decision to make on its own merits, with the
   `solidcore-ops`/`-tess`/`-record` seam rule and the disk budget in view.

Ranked by return: (1) the 14 exact-geometry fixes, (2) GLB export, (3) STEP robustness, (4) kernel
rev bump, (5) plugin host. None of the five needs a brand crate, a licence decision, or a
`ui-kit`-shape rewrite; only (5) needs an IntCAD interface for the wasm host, and only (4) needs a
dependency decision.

### Caveats

- The develop→v0.41.0 **kernel delta is unmeasured** (`8ddce901` is not in this clone). Treat the
  kernel row as "structure known, size unknown".
- Feature statuses in the table are from path/structural evidence plus targeted greps
  (`glb`, `compact`, `mesh_decisions`, `plugins`, `docking`, `tangent_handle`), not from running
  either build.
- `crates/look`'s `build.rs` generates tables into `$OUT_DIR` from masters inside the crate; a
  `look`-free build has no supported path to them, so any route that needs those tables regenerates
  them in IntCAD code.
