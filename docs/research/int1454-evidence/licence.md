# INT-1454 — Licence and provenance of IntCAD's editor UI

Read-only investigation in `/home/hyjing/Code/intgral/intcad`. No build, no tracked-file
change, no push. The ~1,900 untracked `*.sync-conflict-*` files in the working tree were excluded
from every search (`git grep` / `git ls-tree` read Git objects only, so they never enter).

Companion report: `/tmp/opencode/int1454-uikit/reuse-map.md` (called *map* below). Its counts are
mostly right; §6 lists the four claims that need correction.

---

## 1. Licence timeline (verified, not assumed)

| ref | commit | date | `LICENSE-BRAND` | ui-kit / look / theme | root `license` |
|---|---|---|---|---|---|
| IntCAD v0.1.0 | `3b9a9752` | 2026-10-09 | absent | absent (no `ui_kit` either) | `MIT OR Apache-2.0` |
| upstream v0.40.0-rc.2 | `777f9609` (tag) | 2026-11-03 | absent | absent | MIT/Apache |
| common ancestor | `06749ac4` | 2026-11-05 | absent | `crates/ui-egui/src/ui_kit/` 66 files | MIT/Apache |
| upstream v0.40.0-rc.3 | `7001949c` (tag) | 2026-11-07 | absent | `ui_kit` 69 files; no `crates/ui-kit`, no `crates/theme` | MIT/Apache |
| ui-kit extraction | `b34a0ad74` | 2026-11-10 | absent | `crates/ui-kit` created, `license.workspace` | MIT/Apache |
| **last MIT snapshot** | **`a3475683`** | 2026-11-10 | **absent** | `crates/ui-kit` 101 src files, `crates/theme` 4 files | `MIT OR Apache-2.0` (`Cargo.toml:11`) |
| **brand commit** | **`312454f89`** | 2026-11-10 | **added (62 lines)** | `crates/look` created; `LICENSE` files added to ui-kit/theme | still MIT/Apache |
| upstream v0.40.0-rc.4 | `c524fcc6` (tag) | 2026-11-10 | present | — | MIT/Apache |
| upstream v0.41.0 | `3c8efdb5` (tag) | 2026-11-11 | present, byte-identical to `312454f89` | — | MIT/Apache |
| upstream now | `main` `ef415114`, `develop` `481687fff`, tags **v0.42.0, v0.43.0** | — | present | — | MIT/Apache |
| IntCAD fork point (PR #145 base) | `ac560eec` | — | absent | 66-file internal `ui_kit`, no external crates | MIT/Apache |
| IntCAD develop | `02a786d47` (v0.1.1) | — | absent | 66-file internal `ui_kit` | MIT/Apache |

Every licence file was checked at each SHA, not inferred:

* `a3475683`: root has `LICENSE-MIT`, `LICENSE-APACHE`, `NOTICE`; `NOTICE` says "Licensed under
  the MIT License or the Apache License, Version 2.0, at your option". No `LICENSE-BRAND` in the
  root tree. `crates/ui-kit/Cargo.toml` has `license.workspace = true`; workspace licence is
  `MIT OR Apache-2.0`.
* `312454f89` adds exactly `LICENSE-BRAND`, a 9-line brand paragraph in `NOTICE`, and per-crate
  `LICENSE` files; `crates/ui-kit/Cargo.toml` switches `license.workspace` → `license-file`.
* Zero files at `a3475683` and zero in IntCAD's 66-file `ui_kit` carry an SPDX/Copyright header
  (`git grep -lE 'SPDX|Copyright' …` returns 0), so there is no per-file licence to reconcile.
* IntCAD develop contains **no** brand text at all: `git grep -l "Brand and Interface"
  origin/develop` → 0; no `solidcraft_theme` / `solidcraft_look` / `crates/look` references.
* `git merge-base 3c8efdb5 origin/develop` = `06749ac4`, and no upstream commit after the fork is
  reachable from develop. IntCAD has never merged post-brand upstream code.

### §5 verbatim, and what it does and does not say

> **5. Earlier versions**
> Versions of SolidCraft published before this licence took effect stay under the licences they
> were published with.

Plain reading: material already published under MIT/Apache keeps that grant forever; the brand
licence cannot reach backwards. It does **not** say that later commits to the same paths are
MIT, and it does not say what counts as "published" (tagged release vs public commit).

Two MIT bases therefore exist, with different strength:

1. **Release-tag basis.** rc.2 (`06749ac4`) and rc.3 (`7001949c`) are tagged, public releases under
   MIT/Apache that contain `crates/ui-egui/src/ui_kit/**`, `assets/icons/ui/**`,
   `assets/styles/ui/**`, `crates/ui-egui/src/{icons,theme,fonts,toolbar,titlebar}.rs`.
2. **Commit basis.** `a3475683` is a public commit, parent of the brand commit, under MIT/Apache,
   and is the only MIT snapshot of the *extracted* `crates/ui-kit` and of `crates/theme`. Neither
   crate ever shipped in a tag without `LICENSE-BRAND` — check: `crates/ui-kit` and
   `crates/theme` are absent at rc.2/rc.3 and `LICENSE-BRAND` is present at rc.4 onward.

So the safest statement is: **the bytes IntCAD wants were published under MIT/Apache before the
brand licence took effect; the extracted crate layout was published as commits, not as a release.**

---

## 2. Provenance — code

Confidence: **high** = identical blob under MIT/Apache at a cited public SHA; **medium** =
MIT predecessor exists but the file was edited/rewritten; **low** = first published under
`LICENSE-BRAND`.

### 2.1 IntCAD's old UI (the thing to keep developing)

| path | source SHA | licence then | obligations | conf. |
|---|---|---|---|---|
| `crates/ui-egui/src/ui_kit/**` (66 files, 34,203 lines) at `02a786d47`/`ac560eec` | upstream `06749ac4` (MIT/Apache) + 15 IntCAD lines (+6 `fields.rs`, +6 `assistant.rs`, +3 `gallery.rs`) | MIT/Apache | keep MIT notice; Apache NOTICE; state changes | high |
| `crates/ui-egui/src/{toolbar,titlebar,timeline,prefs,home,icons,theme,fonts}.rs` | upstream pre-fork, MIT/Apache | MIT/Apache | as above | high |
| `crates/ui-egui/src/{dfm_*,dfm_review*,project*,agent_chat/**,control*}` (36 files added since `06749ac4`) | IntCAD original | IntCAD's own (today MIT/Apache) | none beyond its own terms | high |
| `assets/icons/ui/**` (66 SVG + `design/*.toml`) | `06749ac4` … `develop` byte-identical | MIT/Apache | keep `ATTRIBUTION.md` rows 163–166 etc. | high |
| `assets/styles/ui/themes/*.json` (25) | `06749ac4`, MIT/Apache | MIT/Apache | attribution rows 207–211+ | high |

Blob proof for the icons — the same 66 SVGs are byte-identical in all four trees:

```
$ comm -12 <(develop flat/) <(06749ac4 flat/) | wc -l        → 66   (66 files each)
$ comm -12 <(45a85b094 flat/) <(3c8efdb5 flat/) | wc -l      → 66
$ diff <(names develop) <(names 3c8efdb5)                    → identical names
```

### 2.2 New `crates/ui-kit` (brand crate) at PR #145 head `f45d9774`

`f45d9774` is exactly v0.41.0 plus 5 lines:

```
$ git diff --stat 3c8efdb5 f45d9774 -- crates/ui-kit
 crates/ui-kit/src/assistant.rs | 5 +++++
$ git diff --shortstat a3475683 f45d9774 -- crates/ui-kit
 39 files changed, 1641 insertions(+), 649 deletions(-)
$ git diff --name-only 3c8efdb5 f45d9774 -- crates/ui-kit   → assistant.rs only
```

* **69 of the 105** files under `crates/ui-kit/src` at `f45d9774` are byte-identical blobs to
  `a3475683` → MIT/Apache, high confidence.
  (The map's "77 of 78 non-test files" is a different denominator; see §6.)
* Files **added** after the brand commit: `src/toast.rs` (363), `src/toast_tests.rs`,
  `src/confirm_tests.rs`, `src/test_support/catalog.de.json`, `LICENSE`. `toast.rs` has no
  pre-brand counterpart → **Brand-only, low**.
* The **edited** files carry Brand-only deltas: `menu.rs` +210/−18, `history.rs` +1/−292,
  `app_bar.rs` +73/−18, `theme_files.rs` +76/−11, `icons/svg.rs` +55/−7, `confirm.rs` +49/−1,
  `choice.rs` +38/−18, `actions.rs` +37/−0, `prefs_window.rs` +26/−2, `container.rs` +18/−0,
  `file_view.rs` +19/−0, `tree.rs` +20/−11, `fields.rs` +24/−5, `history_bar.rs` +16/−107.
  Verified: `panel.rs` and `preferences_figure_data.rs` have **zero** delta since `a3475683`.
* 21 brand-licensed commits touch `crates/ui-kit` after `a3475683` (`ae346a7b2`, `a30b34707`,
  `487a65d0a`, `53a76b23f`, `c1500382e`, …).

### 2.3 `crates/look` — created under Brand, contents moved from MIT

The map says "no pre-Brand snapshot exists → no reuse route". That is **too strong**: the *crate*
is new, but six of its eight source files are moves of MIT files, verifiable by
`git log --follow` and by blob diff against `a3475683`:

| look file | pre-brand source | diff at brand commit | conf. |
|---|---|---|---|
| `src/glyphs.rs` | `assets/icons/ui/source/glyphs.rs` (171 lines) | +35/−9 | high |
| `src/pointer.rs` | `assets/icons/ui/source/pointer.rs` | +3/−2 | high |
| `src/compass.rs` | `assets/icons/ui/source/navigation.rs` | +6/−151 | high |
| `src/fonts.rs` | `crates/ui-egui/src/fonts.rs` | +26/−1389 | high |
| `src/icons.rs` | `crates/ui-egui/src/icons.rs` | +280/−240 | medium |
| `src/themes.rs` | new wiring over `assets/styles/ui/themes/*.json` | new | medium |
| `src/lib.rs` | new wiring (install/install_tables) | new | medium |
| `icons/**`, `fonts/**` | `assets/icons/ui/**`, `assets/fonts/**` | mostly R100 moves | high |

So the correct boundary is **per file**, not per directory: the MIT predecessor is the pre-brand
file, and everything the brand commit added on top (+35/−9 etc.) is Brand material.

`crates/theme` is the same story and better: it existed at `a3475683` under MIT/Apache
(`Cargo.toml`, `README.md`, `src/lib.rs`, `src/palettes.rs`). Brand added `LICENSE`, switched
`license-file`, and changed **4+/1−** lines in `palettes.rs` and **18+/0−** in `lib.rs`.

---

## 3. Provenance — assets

| asset | origin | licence then | obligation | conf. |
|---|---|---|---|---|
| `assets/icons/ui/design/svg/flat/*.svg` (66) | upstream pre-brand, MIT | MIT/Apache | attribution row, MIT notice | high |
| `assets/icons/ui/design/{palette,aliases,references}.toml`, `review.json`, `requested_icons.txt` | upstream pre-brand | MIT/Apache | as above | high |
| `assets/icons/ui/source/{glyphs,markers,navigation,pointer}.rs` | upstream pre-brand (`ATTRIBUTION.md:163–166`) | MIT/Apache; caption geometry adapted from VectorCraft (MIT) | keep both notices | high |
| `assets/styles/ui/palettes.rs` | upstream pre-brand (`ATTRIBUTION.md:206`) | MIT/Apache | attribution | high |
| `assets/styles/ui/themes/*.json` (25) | upstream pre-brand | MIT/Apache | attribution rows 207+ | high |
| IBM Plex Sans/Mono (`assets/fonts/ibm-plex-*`) | IBM, unmodified, `OFL.txt` beside them | SIL OFL 1.1 | ship OFL text; no RFN in IBM Plex, so bundling is fine | high |
| Liberation Sans | Red Hat, unmodified | SIL OFL 1.1 | ship OFL | high |
| BIZ UDPGothic / Craft UI SC / Craft UI TC (Noto subsets) | third party | SIL OFL 1.1 | ship OFL; Noto has no reserved name → rename OK | high |
| `assets/branding/logo/solidcraft-*.svg|png` | upstream, MIT pre-fork | MIT/Apache | **trademark**, not copyright: Apache §6 grants none; LICENSE-BRAND §1(d)/§3(d) claim the marks regardless | med |
| `assets/branding/logo/open-skyfall-mark*` | IntCAD `1b13fdf87` "Rebrand the app as open-skyfall with Intgral's mark (#50)" | IntCAD | none | high |
| `assets/icons/app/**` | app icon; §1(d) brand material | should be IntCAD's own | verify each file | med |

Theme JSON nuance (see §4): the 25 `crates/look/themes/*.json` at the brand commit differ from the
MIT `assets/styles/ui/themes/*.json` by **exactly one added line each** (`"isoparm": …`), e.g.
`blueprint.json` `+1/−0`. All 25 files in IntCAD's replacement `45a85b094` are byte-identical to
those brand-era files, so each carries one Brand-authored line.

---

## 4. The IntCAD replacement commits

`df2dfc19f` ("replace the restricted theme crate") and `45a85b094` ("replace the restricted look
crate"), both on `sync/upstream-v0.41.0`, not yet on develop.

| check | result |
|---|---|
| `df2dfc19f:crates/theme/Cargo.toml` | `license = "MIT OR Apache-2.0"` ✅ |
| `df2dfc19f:crates/theme/LICENSE` | **still the Brand text** ❌ (Cargo no longer references it, but the file is still in the tree) |
| `45a85b094:crates/look/LICENSE` | IntCAD MIT/Apache text ✅ |
| `45a85b094:crates/theme/LICENSE` | still Brand text ❌ |
| `45a85b094:LICENSE-BRAND`, `crates/ui-kit/LICENSE` | still present ❌ — ui-kit has **not** been replaced |
| theme `lib.rs` vs `a3475683` | +205/−514 (derived from IntCAD develop's `theme.rs`, which is MIT) ✅ |
| theme `palettes.rs` vs brand | **+9/−9, 255/258 RGB triples shared (Jaccard 0.98)**; vs `a3475683` +11/−8 → the values are MIT-published, but the file text tracks the Brand version closely ⚠️ |
| look `themes/*.json` | 25 of 26 byte-identical to Brand-era files, 0 identical to the MIT versions; the 26th (`black.json`) is new in `45a85b094` and has no pre-brand counterpart ⚠️ |
| look `glyphs.rs` | Brand delta 27+/50−, and the removed 50 lines are the SolidCraft mark; replaced by Intgral's mark geometry ✅ |
| look `icons.rs` | 25+/233− vs Brand: icon *data* now built from the 66 MIT SVGs ✅ |
| commit-message provenance claim | "IntCAD's v0.1.0 theme code" is **inaccurate**: v0.1.0 `theme.rs` shares 19 % of lines; develop's `theme.rs` shares 80 %. The right citation is `crates/ui-egui/src/theme.rs` at the fork/develop. |

Net: the theme/look replacement is **substantively defensible** (MIT predecessors exist for every
file), with two administrative defects (stale Brand `LICENSE` in `crates/theme`, Brand-era theme
JSONs) and one wrong provenance statement.

---

## 5. What is safe / what is not / what needs sign-off

**Safe without further permission**

* All of IntCAD develop's UI: `crates/ui-egui/src/**` including the 66-file `ui_kit`,
  `assets/icons/ui/**`, `assets/styles/ui/**`, `assets/fonts/**` (OFL terms), IntCAD's 36 added
  files. MIT/Apache permits closed-source distribution, modification and sublicensing; the only
  duties are notice retention (MIT), NOTICE propagation and change statements (Apache-2.0).
* Keeping the old UI and later closing the source. Nothing in MIT/Apache forces publication.
* Reimplementing any brand-era *behaviour* from the pre-brand files, tests and public docs.
* Merging upstream non-brand paths (kernel, engine, io, render, …) — still MIT/Apache at v0.43.0.

**Not safe**

* Shipping `crates/ui-kit` as it stands at `f45d9774`/`45a85b094` in a closed product:
  `LICENSE-BRAND` §3(a)/(b) require written permission to use the kit in another product or to
  distribute it outside SolidCraft. It is still present and still brand-licensed.
* Cherry-picking any upstream commit after `312454f89` that touches `crates/ui-kit`,
  `crates/look`, `crates/theme`, `assets/branding`, `assets/icons/app`.
* Redistributing brand-era theme JSONs verbatim (the `isoparm` line), or the brand-era
  `palettes.rs` text.
* Shipping SolidCraft names/logos/app icons in a commercial product — trademark, independent of
  the code licence (Apache-2.0 §6 grants no trademark rights; §3(c)/(d) reinforce it).

**Needs the rights holder or a lawyer**

1. Whether a public, untagged commit (`a3475683`) is a "version published" under §5, or only the
   tagged releases are (in which case no release ever contained the extracted crates under MIT).
2. Whether §1(a) "the code **and the design**" and §1(c) "design tokens, colour palettes, accent
   ramps" assert a design/trade-dress right that survives the MIT grant on the same bytes.
3. Whether §3(c) ("a look that could be taken for SolidCraft's") reaches an independent CAD app
   that follows ordinary desktop control conventions.
4. Whether IntCAD's own modifications to files that upstream later moved into a brand directory
   remain MIT/Apache (IntCAD's reading: yes — its own edits are its own work).
5. Whether the maintainers will exercise §4 ("the maintainers may license your contribution under
   this licence, and under the MIT License or the Apache License 2.0, at their choice") and
   relicense the kit. This single answer would remove most of the exposure.

---

## 6. Corrections to `reuse-map.md`

1. **`crates/look` "no pre-Brand snapshot exists … no reuse route"** — wrong emphasis. Six of eight
   `look/src` files have MIT predecessors at `a3475683` (§2.3). The directory is brand-licensed;
   the content is largely MIT-derived.
2. **`crates/theme` "MIT at that snapshot"** — correct, but add that `crates/theme` was never in a
   tagged release without `LICENSE-BRAND`; the MIT basis is commits only, not a release.
3. **"77 of 78 non-test files at PR head exist there"** — replace with the exact numbers:
   101→105 files, 69 byte-identical, 39 changed +1641/−649, 5 added (`toast.rs`,
   `toast_tests.rs`, `confirm_tests.rs`, `catalog.de.json`, `LICENSE`).
4. **Missing** (all material): upstream has since released v0.42.0 and v0.43.0, so every later UI
   improvement is Brand, not "open"; `45a85b094` copies Brand-era theme JSONs; `crates/theme/LICENSE`
   is stale Brand text; the commit-message provenance for `df2dfc19f` misstates v0.1.0.

Confirmed from the map: PR head = v0.41.0 + 5 lines; `toast.rs` brand-only; the per-file deltas
for `menu.rs`, `history.rs`, `app_bar.rs`; `panel.rs`/`preferences_figure_data.rs` unchanged;
no per-file SPDX headers; the 66-file IntCAD module is upstream MIT plus 15 lines.

---

## 7. The three scenarios

### (a) A new DFM control on top of the old MIT/Apache implementation

* **The licence text.** MIT/Apache code may be modified and shipped in a closed product provided
  the MIT copyright/permission notice is retained, the Apache `NOTICE` is propagated, and changes
  are stated. New code is IntCAD's. Apache-2.0 §6 grants no trademark rights. `LICENSE-BRAND`
  places no condition on MIT-licensed code (its §5 preserves it), but §1(a) separately claims "the
  design of … controls, panels, menus, ribbon".
* **Engineering.** Build it as new files in `crates/ui-egui/src/ui_kit/` (or the replacement kit),
  using IntCAD tokens from `crates/ui-egui/src/theme.rs`; do not open a brand-era `ui-kit` file as
  a template; keep the diff as new files plus thin registration lines; add a per-file provenance
  header naming the MIT source. Judge the visual result against IntCAD's own screens, not against
  SolidCraft screenshots.
* **Legal.** Whether a control whose *appearance* (not code) follows the brand-era design is
  "Brand Material" under §1(a); whether §3(c) applies to a DFM control that has no SolidCraft
  counterpart.

### (b) Moving a later upstream improvement into the old UI files

* **The licence text.** Commits after `312454f89` to `crates/ui-kit`, `crates/look`,
  `crates/theme`, `assets/branding`, `assets/icons/app` are under `LICENSE-BRAND`, not MIT.
  §5 protects only versions published *before* it took effect, so it does not help here; §4 lets
  the maintainers relicense contributions under MIT/Apache "at their choice", but they have not.
  Non-brand paths stay MIT/Apache.
* **Engineering.** Merge/rebase only non-brand paths. For each UI improvement, read the brand-era
  diff as a bug report, then write the change against IntCAD's MIT file. Never `git cherry-pick`
  or `git checkout <brand-sha> -- <path>` into IntCAD; a merge from upstream would drag the
  brand directories in (`git merge-base 3c8efdb5 origin/develop` = `06749ac4`, so today it would
  import every brand file at once).
* **Legal.** Whether a one-line fix authored after the relicense to a file first published under
  MIT is itself MIT (unlikely — the *fix* is a new copyrightable work under the new licence);
  whether a commit that only touches MIT-usable code but lives under a brand directory is
  directory-scoped Brand material (§1 reads like a path claim).

### (c) Code moved between a restricted directory and an open file

* **The licence text.** The licence attaches to the copy received, not to the current path.
  MIT code moved into `crates/look` stays MIT (§5: earlier published versions keep their licence);
  brand code moved into `crates/ui-egui` stays brand. Paths are evidence, not the licence.
* **Engineering.** Track the source blob `(path@sha, blob, licence)`, not the folder. `git log
  --follow` plus blob comparison is the audit trail (`git diff --numstat <sha>:<old> <sha>:<new>`).
  Where one file mixes an MIT base with brand-only additions, split it before porting; record each
  such file in a `PROVENANCE.md` with the pre-brand SHA. For the sync branch: reset
  `crates/look/themes/*.json` to the `a3475683` MIT text and re-add `isoparm` from IntCAD's own
  palette; regenerate `crates/theme/src/palettes.rs` from `a3475683` rather than from the brand
  tree; delete `crates/theme/LICENSE` (stale Brand text) or replace it with the MIT/Apache texts.
* **Legal.** Whether a file that mixes MIT text with a handful of brand-added lines is a derivative
  of Brand material (probably yes for those lines) and whether they are separable; whether moving
  MIT code into a brand directory is itself a "distribution … other than as part of SolidCraft"
  under §3(b) (it is not, if the code was already MIT, but this is the question to put in writing).

---

## 8. Answer to the strategic question

**Can IntCAD keep developing on the old UI and later close-source?** Yes, on the tree that
develop actually has. Every UI file there descends from MIT/Apache code published before the brand
licence, plus IntCAD's own work; there is no `LICENSE-BRAND` file, no `crates/look`/`crates/theme`
reference, and no post-fork upstream merge. MIT/Apache allow closed-source redistribution with
notice retention. The constraints are: retain `LICENSE-MIT`/`LICENSE-APACHE`/`NOTICE` and
`ATTRIBUTION.md`; state Apache-2.0 changes; ship the OFL texts with the fonts; use only IntCAD
branding; and do not import the brand-era kit, look, theme, or their later edits.

**Highest-risk items, in order**

1. `crates/ui-kit` at the sync head is still brand-licensed — the largest single blocker.
2. `crates/theme/LICENSE` still contains the Brand licence while the crate declares MIT
   (`df2dfc19f`, `45a85b094`).
3. 25 of 26 `crates/look/themes/*.json` byte-identical to the Brand-era files (one Brand-added
   line each); the 26th (`black.json`) is new.
4. The design/trade-dress reading of §1(a)/§1(c) and §3(c) — no code change fixes it.
5. SolidCraft marks still under `assets/branding` in a commercial build (trademark).
6. `palettes.rs` in the replacement tracks the Brand text at 98 %; regenerate from `a3475683`.

---

## Appendix — commands used (non-obvious claims)

```sh
# brand licence history and §5
git log --all --oneline -- LICENSE-BRAND
git show 312454f89:LICENSE-BRAND
git diff 312454f89 origin/develop -- LICENSE-BRAND      # develop has no LICENSE-BRAND
git diff --stat 312454f89 3c8efdb5 -- LICENSE-BRAND     # empty: no wording drift in v0.41.0

# what each release contained
for t in v0.40.0-rc.2 v0.40.0-rc.3 v0.40.0-rc.4 v0.41.0; do
  git cat-file -e $t:LICENSE-BRAND 2>/dev/null && echo "$t brand" || echo "$t mit"
  git cat-file -e $t:crates/ui-kit/Cargo.toml 2>/dev/null && echo "  ui-kit" || echo "  no ui-kit"
done
# → rc.2 mit/no, rc.3 mit/no, rc.4 brand/no, v0.41.0 brand/yes

# IntCAD's old module vs the fork
git diff --stat 06749ac4 origin/develop -- crates/ui-egui/src/ui_kit   # 15 insertions
git merge-base --is-ancestor 3c8efdb5 origin/develop || echo "no upstream merge"
git grep -l "Brand and Interface" origin/develop | wc -l               # 0

# PR #145 head is v0.41.0 + 5 lines
git diff --stat 3c8efdb5 f45d9774 -- crates/ui-kit                       # assistant.rs 5+
git diff --shortstat a3475683 f45d9774 -- crates/ui-kit                 # 39 files +1641 -649

# identical ui-kit blobs / brand-only files
comm -12 <(git ls-tree -r a3475683 crates/ui-kit/src | awk '{print $3}' | sort) \
         <(git ls-tree -r f45d9774 crates/ui-kit/src | awk '{print $3}' | sort) | wc -l   # 69

# look is a move of MIT files
git diff --numstat a3475683:assets/icons/ui/source/glyphs.rs 3c8efdb5:crates/look/src/glyphs.rs  # +35 -9
git diff --numstat a3475683:assets/icons/ui/source/pointer.rs 3c8efdb5:crates/look/src/pointer.rs # +3 -2
git diff --numstat a3475683:assets/icons/ui/source/navigation.rs 3c8efdb5:crates/look/src/compass.rs # +6 -151
git diff --numstat a3475683:crates/ui-egui/src/icons.rs 3c8efdb5:crates/look/src/icons.rs        # +280 -240

# assets: icons and themes
comm -12 <(git ls-tree -r origin/develop assets/icons/ui/design/svg/flat | awk '{print $3}' | sort) \
         <(git ls-tree -r 06749ac4 assets/icons/ui/design/svg/flat | awk '{print $3}' | sort) | wc -l  # 66
git diff a3475683:assets/styles/ui/themes/blueprint.json 3c8efdb5:crates/look/themes/blueprint.json   # +1 "isoparm"
git diff --numstat a3475683:crates/theme/src/palettes.rs df2dfc19f:crates/theme/src/palettes.rs      # +11 -8

# replacement commits
git show --stat 45a85b094
for p in LICENSE-BRAND crates/ui-kit/LICENSE crates/look/LICENSE crates/theme/LICENSE; do
  git cat-file -e 45a85b094:$p && git show 45a85b094:$p | head -1
done
# → LICENSE-BRAND present; ui-kit/LICENSE Brand; look/LICENSE Intcad MIT; theme/LICENSE Brand
```
