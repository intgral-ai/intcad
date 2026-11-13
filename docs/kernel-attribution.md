# Kernel attribution

What the kernel is built from, what this fork changed, whether the historical action-matrix
baseline can judge the Beta run, and what the 213 new failures are. Investigation only: no
upstream issue, comment or PR was posted, and no code was changed.

## Summary

- The fork point is upstream SolidCraft `06749ac46` (2026-11-05, version `0.40.0-rc.2`). The
  fork's first own commit is `238bce2cc`; its head in this worktree is `359cf5849`
  (version `0.1.0-beta.1`), with tag `v0.1.0-beta.1` fixed at `f3285bdb7`.
- The kernel stack is NVIDIA SMLib `v1.0.1` (`3fd5b95`, Apache-2.0), ported by
  `tangency-labs/solidcore` and consumed as two pinned git revisions: `solidcore`
  `8ddce9016` (1.0.1) and `solidcore-record` `f5e8e9a29` (0.1.0).
- This fork changed **no** kernel, naming, face-resolution or record crate code. `crates/kernel`,
  `crates/smlib`, `crates/geom`, `crates/step` and the naming/attachment files are byte-identical
  to the fork point, and both solidcore pins are unchanged. Fork-only work is engine/UI/Mastra
  and snapshot/validation code, not geometry.
- The historical baseline was recorded before the Rust solidcore switch, on the C++ SMLib
  backend. It is **not comparable** to the solidcore Beta run and was never re-blessed. The 213
  failures are therefore a baseline mismatch first, not 213 proven regressions.
- Of the 213: **0 fork-only**, **37 correct refusals triggered by the matrix's synthetic
  parameters**, **88 signature-level upstream-common** (shell status/check and blend-corner
  refusals; representative cases confirmed on upstream `main`), **24 have a later upstream fix
  candidate**, and **64 were not individually classified** (upstream code, later status
  unknown). Details and caveats in [Counts](#4-counts-by-category).
- The plane/face `bad_params` group is 24 entries: 20 are the exact `picked plane does not
  match its exact source face` mismatch; 4 are a different `solid.press_pull` face-class refusal.
  The 20 are upstream doc-layer code. The pre-record-split parent `bd2dc3fad` reproduces them
  with the same `solidcore` pin. A later upstream adapter change is a candidate fix, not a
  proven one.

## 1. Upstream repositories, pins and fork points

### The stack

| Layer | Repository | Revision | Version / licence | Use |
|---|---|---|---|---|
| Original kernel | [NVIDIA-Omniverse/usd-brep](https://github.com/NVIDIA-Omniverse/usd-brep) (`SMLib`) | `v1.0.1`, commit `3fd5b95` (ported from `c9c5979`) | Apache-2.0 | Read-only reference and source of the port; never linked |
| Rust kernel | [tangency-labs/solidcore](https://github.com/tangency-labs/solidcore) | `8ddce901671f1cebd9214a31d21895d725e886c8` | `solidcore` 1.0.1, Apache-2.0 | `Cargo.toml` git dependency behind `solidcraft-smlib` |
| Record | same repository | `f5e8e9a29e0c9b7cf6c4040a5bc8878382b3c62f` | `solidcore-record` 0.1.0, Apache-2.0 | Portable B-rep record, validation, native body file |
| Upstream app | [tangency-labs/solidcraft](https://github.com/tangency-labs/solidcraft) | fork point `06749ac464e16357d0375b78e6b3cac214066493` | 0.40.0-rc.2, MIT OR Apache-2.0 | Source of this fork |
| This fork | [intgral-ai/open-skyfall-desktop](https://github.com/intgral-ai/open-skyfall-desktop) | `359cf584933a88948b3f56f3acc878bfd4ab1197` | 0.1.0-beta.1 | Beta branch `develop` |

`solidcore` is a read-only mirror of the `solidcore/` folder in `tangency-labs/solidcraft`
(its `README.md`). Upstream later stopped using git pins and vendored the same tree into
SolidCraft with `git subtree` in `550848715` (“Kernel: build from the vendored SolidCore and
mirror it back”, #13153). The fork is therefore on the last git-pinned generation of the stack.

### Fork points

- **Open-skyfall fork point:** `06749ac46`, the last upstream commit on the fork's first-parent
  line. First fork-only commit: `238bce2cc` (“AI connections: share BYOK model calls…”).
  `bd2dc3fad` is an earlier upstream commit, not the fork point; it is only used below because
  a parent-revision action-matrix run exists there.
- **Upstream SMLib pin:** `v1.0.1` at `3fd5b95`, recorded in solidcore's `ATTRIBUTION.md`.
- **Record split:** `solidcore-record` enters at `f5e8e9a29` (“Merge pull request #2519 from
  tangency-labs/layer-record”), the record-native layer split. The fork pins that revision
  independently of `solidcore`.

### Commands and output

```sh
# fork point and head
$ git show -s --format='%H %ci %s' 06749ac46 359cf5849 f3285bdb7
06749ac464e16357d0375b78e6b3cac214066493 2026-11-05 00:05:53 +0000 Add the Fair Surface ...
359cf584933a88948b3f56f3acc878bfd4ab1197 2026-11-10 22:15:45 +0000 Docs: record the independent Beta build entry (#1) (#87)
f3285bdb729e2f027abb694f4c97758a4b75353d 2026-11-10 06:09:03 +0000 Release: set the Beta version to 0.1.0-beta.1 (#68) (#78)

$ git show 06749ac46:Cargo.toml | grep solidcore
solidcore = { git = "https://github.com/tangency-labs/solidcore", rev = "8ddce901671f1cebd9214a31d21895d725e886c8" }
solidcore-record = { git = "https://github.com/tangency-labs/solidcore", rev = "f5e8e9a29e0c9b7cf6c4040a5bc8878382b3c62f" }

# pins in the kernel mirror
$ git -C /tmp/opencode/upstream-solidcore show -s --format='%H %ci %s' 8ddce9016 f5e8e9a29
8ddce901671f1cebd9214a31d21895d725e886c8 2026-11-02 20:10:24 -0500 Merge pull request #2475 ...
f5e8e9a29e0c9b7cf6c4040a5bc8878382b3c62f 2026-11-03 06:38:31 -0500 Merge pull request #2519 ...
$ git -C /tmp/opencode/upstream-solidcore describe --contains --all 8ddce9016
tags/last-with-upstream-fixes~277
```

`Cargo.lock` repeats the same two `git+https://github.com/tangency-labs/solidcore?rev=…`
sources with `solidcore 1.0.1` and `solidcore-record 0.1.0`. There is no git submodule and no
vendored solidcore in this fork; `vendor/` holds only the unrelated crates listed there.

The pins are on the mirror's kernel development line, not on its `main` head
(`ef6083597`); the mirror `main` is an ancestor of the pin, and the tag
`last-with-upstream-fixes` is roughly 277 commits after it. This is the same line from which
upstream later vendored the kernel.

### What the fork changed

```sh
$ git diff --name-only 06749ac46..359cf5849 -- \
    crates/kernel crates/smlib crates/geom crates/step \
    crates/doc/src/naming.rs crates/doc/src/persistent_name.rs \
    crates/doc/src/face_anchor.rs crates/doc/src/config_certificates.rs \
    crates/kernel/src/body.rs crates/engine/src/cmd/sketch_face.rs
# (no output: zero files)
```

- **Kernel, naming, face resolution and record code: unchanged.** The copy of
  `crates/kernel/src/body.rs`, `crates/smlib`, `crates/geom`, `crates/step`, `crates/doc`'s
  naming and attachment files, and `crates/engine/src/cmd/sketch_face.rs` is identical to the
  fork point. Both solidcore pins are also identical.
- **Fork-only code is outside geometry:** `crates/engine/src/cmd/snapshot.rs`,
  `crates/engine/src/validation/`, `crates/engine/src/batch.rs`, `crates/engine/src/schema/`,
  `crates/io/src/candidate.rs`, the agent-task crates, the Mastra/UI code, and rebrand text.
  The only change in `crates/engine/src/cmd/mod.rs` registers the new snapshot command
  (`mod snapshot;`, `pub use snapshot::…`, `COMMANDS.extend`). None of it changes the kernel
  operations, face names or face resolution used by the action matrix.
- **Record-native additions:** the snapshot/validation path serialises and validates
  `solidcore_record` data (`crates/io/src/candidate.rs`, `crates/engine/src/validation/*`).
  It consumes the pinned record crate; it does not modify or replace it. The kernel's own
  record export remains `crates/smlib/src/export.rs` (validated at line 1269 with
  `solidcraft_geom::brep::check::validate_built` over `solidcore_record::brep`).
- **Rebrand:** the app is displayed as `open-skyfall`; internal identifiers stay
  `solidcraft`, as required by `docs/compatibility.md`. Rebrand commits change strings and
  appearance authorship, not topology names.

## 2. Is the historical baseline comparable?

**No.** The baseline is from the C++ SMLib migration, not from the Rust solidcore port.

| Dimension | Historical baseline | Beta run | Comparable? |
|---|---|---|---|
| Baseline commit | `ea772a0438b145d7d876eb724b744060fc8c59b6`, 2026-11-02, “Merge develop into main: SolidCraft on the SMLib kernel (v1.1.0-alpha) (#11351)” | `7bf03f6d9` plus shard-15 rerun `1a45733ef` | — |
| Kernel backend | C++ `tangency-labs/smlib`, `SMLIB_REV=a18bbe523b9c7819f1a9da8c40a54c15f9936125` | Rust `solidcore` `8ddce9016` + `solidcore-record` `f5e8e9a29` | **No** |
| Source version | `0.30.1` at the baseline commit | `0.1.0-beta.1` at `359cf5849` | **No** |
| Baseline refresh | Last content change at `ea772a043`; not re-blessed after the solidcore switch | — | **No** |
| Toolchain | `rust-version = "1.95"` recorded; actual compiler unrecorded | `rust-version = "1.95"`; actual compiler unrecorded | Unknown |
| Sharding | 16 cost-cut shards introduced in `c159a3e27` (#1256) | 16-shard release-gate run | Same in kind; only timeout outcomes depend on it |
| Config | validity/unfold/edit sampling not recorded with the file | release-gate defaults | Unknown |

```sh
$ git show -s --format='%H %ci %s' ea772a043
ea772a0438b145d7d876eb724b744060fc8c59b6 2026-11-02 22:53:32 +0000 Merge develop into main: SolidCraft on the SMLib kernel (v1.1.0-alpha) (#11351)
$ git show ea772a043:Cargo.toml | grep -m1 '^version'
version = "0.30.1"
$ git show ea772a043:crates/smlib/SMLIB_REV
a18bbe523b9c7819f1a9da8c40a54c15f9936125
$ git diff --quiet ea772a043..359cf5849 -- tests/action_matrix/baseline.json && echo unchanged
unchanged
```

The baseline is stale even in its own history: issue #55 records that its
`partial_face_contact_pattern` entries were blamed to `c159a3e27` (2026-10-14), before the
migration, and were wrong in both directions afterwards. The migration merge `ea772a043`
rewrote some entries but did not re-run the whole matrix on the final backend.

**Conclusion:** the 213 actions are a ratchet mismatch. They must not be called regressions or
counted as pass until a full matrix is recorded on the solidcore backend. The historical
backend was not re-run, and upstream `main` was not run here: those are **unknown**, not pass.

## 3. Representative cases

Action identities are exactly as they appear in
`/tmp/opencode/action-new-failures.json` (213 entries). Messages are shortened.

| # | Action identity (design · entity · action) | Signature | Upstream reproduction / later upstream | Classification |
|---|---|---|---|---|
| 1 | `corpus/issue-616-lid-ribs` · `Lid/F10:+x` · `face.shell` | `SmLoop::Test #2 Bad Loop Orient … loop may have wrong orientation [owner SmLoop]` | Still named in upstream `main`'s [`tests/KNOWN_FAILING_TESTS` at `3b561df76`](https://github.com/tangency-labs/solidcraft/blob/3b561df768c5ea95821ccdda9b74e816e0fbe376/tests/KNOWN_FAILING_TESTS): “2 shells (Bad Loop Orient on issue-616-lid-ribs; status 1001 on filleted-shelled-box)”. No fixing commit found. | **Upstream-common limitation** (confirmed still on upstream `main`) |
| 2 | `oracle/42-overlapping-patterns` · `Plate/F2:start\|F4:side:c1@F5.11#1` · `edge.chamfer` | `not supported yet: the chamfer meets a corner of this kind, which SMLib's blends do not handle yet` | The same refusal is still in upstream `main` at `crates/kernel/src/blend_smlib.rs:458`. No fixing commit found. | **Upstream-common limitation** (refusal path still present) |
| 3 | `oracle/17-loft` · `Loft/F4:end\|F4:side:l1` · `edge.fillet` | `SMLib's result is not a closed solid (tangent propagation blended the picked edges' tangent chain …)` | No commit found that names this loft or this result. Upstream `main` still lists several fillet status-1001/variable-fillet failures in `tests/KNOWN_FAILING_TESTS`; this exact action was not re-run upstream. | **Upstream-common at the pin; later status unknown** |
| 4 | `oracle/63-hinge-knuckle` · `Hinge/F2:side:l2` · `face.draft_neutral` | `SMLib draft: operation: SMLib's result is not a closed solid` | No fixing commit found for the draft result; upstream's `oracle/63` ignored test is about a knuckle round, not this draft. Not re-run upstream. | **Upstream-common at the pin; later status unknown** |
| 5 | `demo/pi-case` · `Body1/F4:inner:F2:side:l1` · `face.extrude` (also `face.sketch`, `face.offset_plane`, `face.press_pull`) | `picked plane does not match its exact source face; choose a supported exact face or explicitly use fixed:true` | **Yes**, on upstream `bd2dc3fad` with the same `solidcore` pin, but before the `solidcore-record` split: `/tmp/opencode/pi-case-parent.jsonl` records the identical `bad_params` for `F4:inner:F2:side:l1/l2` on both `demo/pi-case` and `demo/pi-case-0.8`. Later upstream [`1a4f3e4a6`](https://github.com/tangency-labs/solidcraft/commit/1a4f3e4a6f2823f7548b9517f3b83df9457ba99e) and [`94ce45a1f`](https://github.com/tangency-labs/solidcraft/commit/94ce45a1f263bb55de05d574ca49f4ee6ac02afc) make `planar_face` accept flat B-spline faces; that is a candidate fix for this class, not individually verified against these actions. | **Later-fix candidate** (upstream-common at the pin) |
| 6 | `oracle/42-overlapping-patterns` · `Plate/F2:start` · `face.extrude` (also `face.press_pull`) | `SMLib export: the mapped B-rep does not validate: portable B-rep: a closed shell bounds two regions and encloses one` | The pinned record validator is `solidcraft_geom::brep::check::validate_built` in `crates/smlib/src/export.rs:1269`. Upstream `main` now validates with `solidcore_ops::record_check::validate_built` (`crates/smlib/src/export.rs:1299`), a later record-native change. Not individually re-run. | **Upstream-common at the pin; later validator change is a candidate fix** |
| 7 | `regressions/partial_face_contact_pattern` · `Body1/F1:+x` · `face.extrude` (also `face.press_pull`; `+z` pair) | `SmBrep::Test #12 Bad Inner shells: NonInfiniteRegion Inner shell is not geometrically inside its containing outerShell` | This is [#55](https://github.com/intgral-ai/open-skyfall-desktop/issues/55). Upstream solidcore `main` now has [`1ee2ecc1c`](https://github.com/tangency-labs/solidcore/commit/1ee2ecc1c0023052d9d250faf6efe071791c3ddd) (`put_enclosing_shells_first`, in `crates/solidcore/src/topology/brep/enclosing.rs`), absent from the pinned `8ddce9016`. Upstream `main` also calls it from the flat-array import. | **Later-fix candidate** (root fix exists upstream; fork pin lacks it) |

For every kernel signature above, the fork has no code diff in the path that produces it. A
failure therefore reproduces at the fork point by construction; only case 5 has a separate
parent-revision run in this evidence set.

## 4. Counts by category

The run evidence is release-gate run `42790743092` at `7bf03f6d9`, plus the shard-15 rerun at
`1a45733ef`: 54029 actions and 213 failures relative to the historical baseline.

### By failure signature (all 213)

| Count | Signature group |
|---:|---|
| 48 | shell operation: `SMLib failed (status 1001)` |
| 23 | shell: `the N mm walls would meet along a line` (correct refusal of the matrix's 0.1×size wall) |
| 20 | shell: SMLib's check refuses the result (`Bad Loop Orient`, `Bad Inner shells`, …) |
| 20 | **plane/face mismatch** (`picked plane does not match its exact source face`) |
| 20 | blend corner unsupported (`the chamfer/fillet meets a corner of this kind`) |
| 14 | blend size too large (`the largest … that fits is N mm`) |
| 9 | blend result not a closed solid |
| 8 | boolean check refusal (`SMLib boolean: SMLib's check refuses …`) |
| 8 | naming/reference lost (`the reference … no longer exists as it was picked`) |
| 7 | shell unsupported (`not supported yet: SMLib shell for this geometry`) |
| 7 | blend status 1001 |
| 6 | draft result not a closed solid |
| 5 | blend check refusal |
| 4 | `solid.press_pull`: `that face is not a fillet's or a chamfer's` |
| 4 | export/record validation |
| 4 | face pick failure (`no face at [x,y,z]`) |
| 3 | draft unsupported |
| 1 | face pick ambiguous (`equally near to two edges or faces`) |
| 1 | timeline error on an edited design (boolean check refusal) |
| 1 | design load failure (variable fillet status 1001) |
| **213** | |

Command totals: `solid.shell` 100, `solid.chamfer` 41, `solid.press_pull` 14,
`solid.fillet` 14, `solid.draft` 13, `solid.extrude` 10, `solid.hole` 7,
`construct.plane.offset` 5, `sketch.create` 5, `construct.plane.perpendicular` 2,
plus one edit timeline error and one load failure. Designs: 37; the largest groups are
`corpus/modern-plastic` (27), `oracle/42-overlapping-patterns` (24),
`regressions/loft_cut_beside_round_hole` (14), `demo/pi-case` (12) and
`oracle/34-chamfer-then-fillet` (10).

### By provenance (all 213)

| Count | Class | Basis |
|---:|---|---|
| 0 | **Fork-only regression** | Zero diff in kernel/naming/face-resolution/record paths; identical solidcore pins. |
| 37 | **Test-environment / baseline difference** | 23 shell walls-touch + 14 blend-too-large. Both are correct refusals of the matrix's synthetic 0.1×size parameter, not wrong geometry; the C++ baseline recorded them as passes. |
| 88 | **Upstream-common, signature confirmed at upstream `main`** | 48 shell status 1001 + 20 shell check refusal + 20 blend corner unsupported. Upstream `main`'s `KNOWN_FAILING_TESTS` names representative shell failures, and its `blend_smlib.rs` still contains the corner refusal. This is confirmation of the signature/class, not of every entity. |
| 24 | **Later upstream fix candidate** | 20 plane/face mismatch (`planar_face` changes `1a4f3e4a6`/`94ce45a1f`) + 4 #55 multi-lump boolean actions (solidcore `1ee2ecc1c`). Not individually re-run on upstream `main`; this is a mechanism match, not a proven fix. |
| 64 | **Upstream-common or insufficient evidence** | 9 blend not-closed, 8 naming, 7 shell unsupported, 7 blend status 1001, 6 draft not-closed, 5 pick, 5 blend check, 4 press_pull face-class, 4 non-#55 boolean, 4 export, 3 draft unsupported, 1 timeline, 1 load. Code is upstream, but no later fix or individual reproduction was established. |
| **213** | | |

These are counts **by message signature and source evidence**, not 213 individual
reproductions. A signature classified “upstream-common” means the same code path is in the
upstream revision; it does not claim every entity would fail identically on upstream `main`.

### The plane/face group and the load failures

The 24 `bad_params` entries are two different things:

| Count | Action | Message | Designs |
|---:|---|---|---|
| 5 | `face.extrude` | `invalid parameters for face profile: picked plane does not match…` | `demo/pi-case` (2), `demo/pi-case-0.8` (2), `oracle/47-draft-curved-pocket` (1) |
| 5 | `face.offset_plane` | `invalid parameters for construct.plane.offset: picked plane does not match…` | same |
| 5 | `face.sketch` | `invalid parameters for sketch.create: picked plane does not match…` | same |
| 5 | `face.press_pull` | `invalid parameters for face profile: picked plane does not match…` | same |
| 4 | `face.press_pull` | `invalid parameters for solid.press_pull: that face is not a fillet's or a chamfer's` | `corpus/modern-plastic` |

So the plane/face mismatch is **20**, not 24. The other 4 are a face-class refusal on blend faces,
not a plane resolution failure.

Load failures: **1**. `oracle/77-fillet-variable-linear` fails while loading, at
`step 8 (solid.fillet): SMLib variable fillet: operation: SMLib failed (status 1001)`. That is
a variable-fillet backend limitation, not a design-file or fork deserialisation failure. The
one `timeline_error` is an edited `oracle/27-sketch-trapezoid` boolean check refusal, not a
load failure.

## 5. Plane/face mismatch: focused source investigation

### Exact path

At the fork, a face pick goes through two different exact-plane answers:

1. `crates/engine/src/cmd/sketch_face.rs:162` (non-fixed pick) builds `picked` from
   `body.body.face_plane(index).or_else(|| body.body.planar_face(index))`.
2. `crates/kernel/src/body.rs:725` `face_plane` prefers the **feature's recorded source plane**
   (`ExtrudeSide`, `ExtrudeCap`, `ShellPlanar`, `Box`, `CylinderCap`, …); only `Imported`
   faces fall through to `planar_face`.
3. `crates/kernel/src/body.rs:743` `planar_face` returns only `Surface::Plane` and
   `Surface::PlaneThrough` at the pin; a B-spline or other surface returns `None`.
4. `crates/doc/src/face_anchor.rs:193-209` `capture_face_anchor` then gets the anchor plane from
   `anchor_face_plane` (`crates/doc/src/config_certificates.rs:812`). For a
   `FaceCertificate::Lineage` target that is `lineage_frame` over **`planar_face`**
   (`config_certificates.rs:784`), not `face_plane`.
5. The guard at `crates/doc/src/face_anchor.rs:208` requires
   `picked.normal·canonical.normal ≥ 1 − 1e-7` and an absolute origin-to-plane distance
   `≤ 1e-7`; otherwise the command fails with the observed message.

The mismatch is therefore a **dual-source exactness defect in the doc/kernel adapter**, not in
the record layer and not in the fork:

- `face_plane` can return a feature's stored source plane, while `planar_face`/`lineage_frame`
  can return the current exact plane (or `None`). The two are not guaranteed to agree after a
  later operation, a shell/offset, or a B-spline face.
- The fork has no diff in any of these files. `crates/doc`'s naming files are also unchanged.
- The parent revision `bd2dc3fad` (same `solidcore` `8ddce9016`, but before the
  `solidcore-record` split; 98 lines differ across these four plane files) reproduces the exact
  `bad_params` on `demo/pi-case` and `demo/pi-case-0.8`; see
  `/tmp/opencode/pi-case-parent.jsonl`. That is direct upstream-revision evidence that the
  failure predates both the fork and the record split. It does not by itself prove the current
  upstream `main` still fails.

### Later upstream and candidate fix

Upstream `main` has two related changes, both after the fork point:

- [`1a4f3e4a6f2823f7548b9517f3b83df9457ba99e`](https://github.com/tangency-labs/solidcraft/commit/1a4f3e4a6f2823f7548b9517f3b83df9457ba99e) —
  `Body::planar_face` also accepts a flat B-spline/rational B-spline face whose control net lies
  on one plane (#14346).
- [`94ce45a1f263bb55de05d574ca49f4ee6ac02afc`](https://github.com/tangency-labs/solidcraft/commit/94ce45a1f263bb55de05d574ca49f4ee6ac02afc) —
  `planar_face` delegates the flat-face test to solidcore-ops (`Body::face_plane_normal`, SMLib
  `IsPlanar`) instead of local control-net maths (#14509).

Those remove the `planar_face = None` case for flat B-spline faces and make its answer come from
the kernel. They do **not** change the `capture_face_anchor` comparison or the `face_plane` vs
`planar_face` split; the message still exists at upstream `main`'s
`crates/doc/src/face_anchor.rs:263`. So:

- **Classification:** upstream-common at the pin; the later adapter work is a plausible fix for
  the 20, but it is not proven for these exact entities without a run on upstream `main`.
- **Minimal candidate fix (not implemented):** make the capture use the same plane source as
  the pick. Either pass the selected `exact_plane` into `capture_face_anchor` and anchor in it,
  or, for a `Lineage` certificate, resolve both the pick and the anchor through `planar_face`;
  if the two still differ, compare with a tolerance scaled by the body size rather than the
  absolute `1e-7`. This changes what an attachment follows, so it needs an upstream design
  decision and a regression test; a broad refactor is not justified.
- **Not the cause:** the fork's record-native snapshot/validation code never participates in
  this path, and the pinned `solidcore-record` is only the `Plane` type here.

## 6. #55 multi-lump union, #56 NURBS refusal, and PR #60

### #55 — multi-lump union (`regressions/partial_face_contact_pattern`)

- **Not fork-introduced.** The failure is the upstream `kernel::boolean(..., Union)` path in
  `crates/kernel/src/ops.rs`, with the same pinned solidcore. The fork's kernel files are
  identical to the fork point; issue #55 reproduces at `develop` `1b13fdf87`, where the kernel
  was unchanged since `9df9d1fbd`.
- **Upstream root fix exists after the pin.** Solidcore `main` has
  [`1ee2ecc1c0023052d9d250faf6efe071791c3ddd`](https://github.com/tangency-labs/solidcore/commit/1ee2ecc1c0023052d9d250faf6efe071791c3ddd)
  (`put_enclosing_shells_first`), which puts a bounded region's enclosing shell first so SMLib
  does not read a disjoint lump as a void. It is absent from pinned `8ddce9016`; upstream
  `main` now calls it from the flat-array import and solidcore-convert.
- **Is PR #60 the right minimal fix?** It is a narrow, credible **in-repo workaround**: 125
  lines in `crates/kernel/src/ops.rs` plus engine/kernel history tests, no new mesh decision
  site, and it unions a multi-lump first operand one piece at a time. It is not the root fix
  and it is stacked on open PR #53 (`fix/kernel-cases-exempt-snapshot`), so its base must be
  retargeted/rebuilt on `develop` before merge. The cleaner long-term route is a solidcore pin
  bump (or the vendored-subtree swap) to a revision containing `1ee2ecc1c`; [#60](https://github.com/intgral-ai/open-skyfall-desktop/pull/60)
  is acceptable only as a deliberate fork-local stopgap. Do not merge it as-is.

### #56 — 1e5-offset sphere NURBS refusal

- **Not fork-introduced; not fixed upstream.** The failure is in SMLib/solidcore's
  `SmBSplineSurface::Test #5 Bad NonMonotonic CPts` check on a translated sphere. Upstream
  `main` still marks the dedicated test ignored as “kernel bug, [solidcore#3131](https://github.com/tangency-labs/solidcore/issues/3131)”
  (`crates/kernel/tests/boolean_props.rs`, `disjoint_sphere_box_union_moved_far`), and the
  check is still in solidcore. Issue [#56](https://github.com/intgral-ai/open-skyfall-desktop/issues/56)
  is exposed by the SMLib migration `9df9d1fbd`.
- **Is PR #60 the right minimal fix?** No. PR #60 is about multi-lump unions and does not touch
  the NURBS control-net check. The current mitigation is [#57](https://github.com/intgral-ai/open-skyfall-desktop/pull/57)'s
  expiring known-failure exception, not a kernel fix. A durable fix belongs in solidcore (or a
  deterministic minimal regression test once the kernel accepts it). #56 is not one of the 213
  action-matrix entries; it is a separate property-test failure.

## 7. Remaining work / not attempted

- No upstream issue, comment or PR was created; no PR was merged or pushed.
- No full workspace build and no full action matrix run. The 54029-action run and its 213
  failures are taken from the supplied evidence (`/tmp/opencode/action-matrix-report.json`,
  `/tmp/opencode/action-new-failures.json`).
- No action-matrix run was made on upstream `main`, and the historical C++ SMLib backend was not
  re-run. Results there are **unknown**, not pass.
- The 213 were classified by signature and source evidence, not individually reproduced. A
  full re-bless on the solidcore backend is the required next step; until then the ratchet is
  invalid.
- No pin bump, no `planar_face`/`capture_face_anchor` change, no #55 or #56 kernel fix was
  implemented. PR #60 and PR #57 were read, not modified.
- The `press_pull` “not a fillet’s or a chamfer’s” group (4) needs a separate action-matrix or
  face-class decision; it is not part of the plane/face mismatch.

## Cylinder/Sphere boolean refusal (Beta.3 property lane)

The Beta.3 property lane hit one exact SMLib refusal that is not in the 213 action-matrix
ratchet:

- Input: cylinder `at [22.5, 12.5, 2.5], axis X, r 11.25, h 5`; sphere `at [10, 2.5, 0], r 12.5`;
  operation union, both operand orders, uniform scales 0.05/1/40 and translations
  `off 0/1e3/1e5`.
- SMLib returns `SMLib boolean: SMLib's check refuses the result: ... SmFace::Test #12
  Face->Vertex ... SmVertex::Test #1 Bad Vertex Gap ... offset=12.5, tolerance=2e-05`.
- The sphere's rightmost point `(22.5, 2.5, 0)` is inside the cylinder's start cap, so the two
  solids touch at exactly one interior point. Their union is not a manifold solid; the checker
  is refusing an invalid topology, not a valid no-volume union.
- Transform matrix, measured at the pin: union refused at 1x, 0.05x, offset 0 and offset 1e3
  (the offset stays 12.5); union built at 40x (volume `650,833,278.09`, normalised `A+B`) and
  offset 1e5 (volume `A+B`). Intersect is empty and cut is the cylinder at every transform. The
  built bodies pass `validity()`. No input body changes, and the engine preview leaves the
  document unchanged.
- The fork's `crates/kernel`, `crates/smlib`, and both solidcore pins (`8ddce9016`,
  `f5e8e9a29`) are unchanged from the fork point; the same code is on upstream/develop. This is
  upstream-common at the pin, not a fork-only regression.
- The three property tests (`commutative`, `identity_on_grid`, `scale_invariant`) share the
  minimal input. `crates/kernel/tests/boolean_props.rs` accepts only that exact relative
  geometry, operand order, uniform scale and translation, and the exact error text; a
  different sphere/cylinder/error, transform or built volume fails. The deterministic
  `cyl_sphere_refusal_is_the_exact_reviewed_upstream_case` asserts the refusal, the transform
  outcomes, `A+B` volume and the input invariants; if SMLib starts behaving differently, that
  test fails and the exception must be removed.
- No upstream issue/comment was posted. This is recorded for later tracking; a kernel fix is out
  of this Beta's scope.

## Cylinder/Sphere seam refusal (Beta.3 property lane)

A second exact case is a boolean seam limitation, not a point tangency:

- Class: an axis-X cylinder and a sphere. The generator hit e.g. cylinder
  `at [0, 17.5, 17.5], axis X, r 7.5, h 12.5` with sphere `at [0, 25, 27.5], r 11.25`, and
  cylinder `at [17.5, 10, 0], axis X, r 10, h 5` with sphere `at [7.5, 20, 0], r 11.25`;
  operations intersect and cut; uniform scales 0.05/1 and translations `off 0/1e3/1e5`.
- Union builds a sound body. Intersect and cut are refused at 1x and 0.05x with
  `SMLib boolean: SMLib's check refuses the result: ... SmFace::Test #29 Face crosses Seam:
  Face boundary illegally crosses a surface seam. [owner SmFace]`.
- At offset 1e5 all three operations can hit the reviewed #56 `SmBSplineSurface::Test #5 Bad
  NonMonotonic CPts` (`[owner SmSphere]`) refusal. The operation fails safely; no body is
  returned and no input body changes.
- The predicate binds that primitive pair, the two operations and the exact checker error, in
  either operand order; a different primitive pair, operation or error fails. The deterministic
  `cyl_sphere_seam_refusal_is_the_exact_reviewed_upstream_case` asserts the refusals and the
  union/scale/translation paths. This is upstream-common at the pin; a kernel/checker fix is out
  of this Beta's scope.

## Evidence index


The `/tmp/opencode/...` paths are local investigation artifacts, not files committed to this
repository.

- 213 failures: `/tmp/opencode/action-new-failures.json`; full run:
  `/tmp/opencode/action-matrix-report.json` (54029 actions).
- Parent-revision plane/face reproduction: `/tmp/opencode/pi-case-parent.jsonl` (upstream
  `bd2dc3fad`, same `solidcore` pin, before the record split).
- Baseline: `tests/action_matrix/baseline.json` at `ea772a043`; unchanged through
  `359cf5849`.
- Fork-point and pin proofs: commands in sections 1 and 2.
- Upstream fixes read: `1a4f3e4a6`, `94ce45a1f` (plane/face candidate); `1ee2ecc1c` (#55 root
  fix); upstream `main` `3b561df76` (`KNOWN_FAILING_TESTS`, `boolean_props.rs`).
- Cylinder/Sphere property refusal: run `43381698526`, tested merge `13417bb37`; the property
  JUnit and the deterministic test above.
