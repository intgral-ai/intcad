# INT-1454 Phase 0 — licence and asset audit (initial)

Scope: what may be adopted from SolidCraft v0.44.1, what may not, and what evidence each
conclusion rests on. This is the Phase 0 draft; every adopted unit must add its own row to the
upstream-sync ledger with its licence verdict.

## 1. Licence map at v0.44.1

| File | Covers | Verdict |
| --- | --- | --- |
| `LICENSE-MIT` | the repository, unless stated otherwise | adoptable |
| `LICENSE-APACHE` | same, at the user's option | adoptable |
| `LICENSE-BRAND` | brand and interface material, §1 | **not adoptable** without written permission |
| `NOTICE` | copyright, brand carve-out, third-party pointers | must be carried/adapted |
| `THIRD-PARTY-LICENSES.txt` | generated dependency licences | regenerate, never hand-edit |
| `solidcore/LICENSE`, `solidcore/NOTICE`, `solidcore/ATTRIBUTION.md` | the vendored kernel (Apache-2.0; SMLib/SM_API port of USD BRep) | adoptable with notices |

`LICENSE-BRAND` §5: *"Versions of SolidCraft published before this licence took effect stay under
the licences they were published with."* IntCAD's UI code and assets descend from the pre-brand
snapshot, so they remain MIT/Apache-2.0/OFL. That carve-out covers **what IntCAD already has**,
not new upstream brand changes.

## 2. Brand boundary at v0.44.1 (measured)

| Path | Files | Size | Status |
| --- | --- | --- | --- |
| `crates/ui-kit` | 115 | 58,976 lines | Blocked (LICENSE-BRAND §1a) |
| `crates/look` | 455 | 15,202 text lines + 2 TTF (icon masters, themes) | Blocked (§1b) |
| `crates/theme` | 5 | 2,677 lines | Blocked (§1c) |
| `assets/branding` | 10 | 84 text lines + 4 PNG | Blocked for new content (§1d) |
| `assets/icons/app` | 18 | 173 text lines (svg/json/rs) + 15 binary (png/ico/icns/car) | Blocked for new content (§1d) |
| `assets/icons/ui` | **absent at v0.44.1** | — | upstream moved UI icons into `crates/look/icons` |

≈ 603 files, **77,112 text lines** (sum of the rows; binaries excluded), are unavailable on
licence grounds. Third-party material
inside those folders keeps its own licence (IBM Plex fonts, OFL 1.1), but that does not open the
surrounding brand material.

**Provenance split (matters for §5):** most of that material predates the licence. At the
pre-brand snapshot `a3475683` the UI kit is 103 files / 50,748 lines and has no `LICENSE-BRAND`;
the brand-taking commit is `312454f8` (2026-11-10, "Look: … as the shared crate"). Per file:
ui-kit 104/115, look 440/455, theme 5/5, branding 10/10, app icons 18/18 were added before it.
Growth after the licence: +12 files / +8,228 lines. IntCAD ships the pre-brand material it forked
(it is what `crates/ui-egui/src/ui_kit` copies); what it cannot take is the *later* growth.

**Licence-uncertain, not yet cleared:** post-brand changes to `ui-egui` chrome files that
implement the components §1(a) names (`titlebar.rs`, `toolbar.rs`, `prefs*`, `shell.rs`,
`workspace.rs`, `documents.rs`, `starting.rs`, `home.rs`, `navigation_bar_view.rs`). They carry the
workspace MIT/Apache licence but their content is the licensed interface; §5 grandfathers only
versions published before the licence. Default position: do not reuse until the owner/legal
answers CD-6.

Consequence: every upstream UI-icon and theme change since the fork is **Blocked**, and IntCAD
must draw any new icon itself. This is a UI-completeness gap, allowed by the Spec to be recorded
rather than closed by importing brand assets.

## 3. What IntCAD already ships, and why it stays

- `crates/ui-egui/src/ui_kit/**` (66 files) is an in-crate copy from the pre-brand snapshot, not
  the upstream `ui-kit` crate (which did not exist at the fork point `06749ac4`).
- `assets/icons/ui/**` (400 files) is byte-identical to `06749ac4` except
  `design/requested_icons.txt` and `source/glyphs.rs`
  (`git diff --stat 06749ac4 origin/develop -- assets/icons/ui`).
- `assets/branding` holds IntCAD's own `open-skyfall-*` marks plus pre-brand SolidCraft marks.
- IntCAD has **no** `LICENSE-BRAND`; upstream copyright notices stay in `NOTICE`.

## 4. Adoption rules per unit

1. Code under MIT/Apache (kernel, solidcore, doc, engine, io, viewport, control, plugins, MCP)
   may be adopted, adapted and modified; keep the licence texts and record modifications.
2. Anything inside the §1 brand paths is not adopted. A unit that *needs* a new icon, palette or
   look file is recorded as a UI gap, never closed by copying.
3. Vendoring `solidcore/` brings its own `LICENSE`, `NOTICE`, `ATTRIBUTION.md`; they must stay
   byte-identical and `NOTICE` must describe the new kernel arrangement.
4. New third-party crates (dependency bumps) go through `cargo xtask licences --check` and
   `cargo xtask assets`; `attribution/pending/<slug>.md` gets the rows, never `ATTRIBUTION.md`
   directly.
5. Screenshots and recordings in upstream `docs/screenshots` are not adopted; IntCAD captures its
   own evidence (repo rule).

## 5. Audit defects found in the current tree

- `NOTICE` says the kernel "builds on the truck crates", while `crates/kernel/Cargo.toml` depends
  on `solidcore-record` and `solidcraft-smlib`. The notice must be corrected in the kernel unit;
  this is pre-existing drift, not a sync result.
- `cargo xtask licences --check` covers Cargo dependencies only. npm/Node material (Mastra
  packaging) and rights clearance are not covered by it; the Spec's licence review for those is a
  human/legal step, and no such material is in this sync's scope.

## 6. Open items

| Item | Why open | Next step |
| --- | --- | --- |
| Upstream UI icons/themes needed by a migrated feature | brand-blocked | record as UI gap; draw in-house |
| `solidcore/` attribution layout when vendored | vendoring changes the NOTICE story | verify files byte-identical after adoption |
| Third-party fonts inside `crates/look/fonts` | OFL allows use, but the folder is brand-scoped | do not import; keep IntCAD's existing OFL fonts |
