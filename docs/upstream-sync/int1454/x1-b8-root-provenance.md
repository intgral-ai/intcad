# X1 / B8 / K15: root guidance, notices and asset provenance

Scope: the nine surviving root names assigned to B8/K15 in the
[X1 crosswalk](x1-mixed-path-crosswalk.md). Compare pre-sync IntCAD
`0fd92bea2`, frozen upstream `e151d5ea`, and the isolated candidate
`db0b8dd35`. Source identity establishes lineage, not approval to copy or
publish. This audit does not change any of the nine files.

| Root path | Baseline / upstream / candidate blob prefixes | Evidence and disposition |
| --- | --- | --- |
| `AGENTS.md` | `19c013e8cac7` / `ccf25b6ec5a7` / `19c013e8cac7` | **B8 / K15 — baseline retained.** The candidate still documents the local `xtask layers` gate. Keep IntCAD's worktree and licence rules; a future layering update must describe the actual candidate gate, not copy upstream instructions wholesale. |
| `README.md` | `5db8f208b72b` / `a987881d5d3e` / `5db8f208b72b` | **B8 — baseline retained.** IntCAD identity and links are retained (branding hotfix `0ecba55cc`). The upstream README links its sponsor banner; the candidate README does not. No sponsor placement or publication change is proposed. |
| `CONTRIBUTING.md` | `0b0449b7c454` / `13d12a5514ce` / `0b0449b7c454` | **B8 — baseline retained.** The candidate preserves IntCAD contribution guidance (`0ecba55cc`). Any future update should adapt the rules to the fork's workflow, not adopt upstream publication or CI policy by implication. |
| `ATTRIBUTION.md` | `a47b86fa39ae` / `ff3feba76647` / `302cade96646` | **B8 / K0 — scoped candidate update.** The baseline-to-candidate diff replaces the fetched-crate description with the vendored kernel source, use and Apache-2.0 notice pointers. The sponsor attribution remains in place. Verify the final distributed notices and asset scope separately; no general rights clearance follows. |
| `NOTICE` | `6ce13cc1d3bb` / `df7945073abd` / `48c37493d633` | **B8 / K0 — scoped candidate update.** The IntCAD notice replaces the former truck kernel paragraph with the vendored SolidCore, SMLib/SM_API and USD BRep credit; the surrounding IntCAD notice is retained. Confirm that the final packaged dependency inventory and notice files match what is actually shipped. |
| `assets/README.md` | `f17f4190564d` / `b3d8504f7b26` / `f17f4190564d` | **B8 / CD-6 — baseline retained.** The local asset guide survives unchanged (`0ecba55cc`); the upstream path/layout update is not evidence for the origin of any candidate UI file. No asset reuse or brand import authorized. |
| `docs/layers.md` | absent / `64f5b9572ea8` / absent | **K15 / B8 — root guide not adopted.** The upstream root guide originated in `097ea3bb0` and later describes the path-level seam checks. The candidate has a distinct vendored `solidcore/docs/layers.md` (`c8b487a9c4ad`) and a local root `docs/architecture.md`, not the upstream root guide. See the K15 evidence below. |
| `docs/mutation-testing.md` | `e5bbd969ae63` / `e5bbd969ae63` / `e5bbd969ae63` | **B8 / K0 — inherited source, no new root import.** Root history traces to `bc68e14c3`. The vendored guide is a distinct blob (`2739a3f12616`). The unchanged guide and its historical pilot results do not establish a current IntCAD mutation run. |
| `docs/sponsors/blacksmith-ci-powered.png` | `d21e27dd272a` / `d21e27dd272a` / `d21e27dd272a` | **B8 — inherited third-party asset, not new migration material.** `66951921a`, an ancestor of the baseline, added the banner and attribution together. The root attribution records sponsor-supplied permission for sponsor credit only, outside the application. No new use: any new placement, redistribution, or application packaging needs owner/rights review for that use. |

The vendored `LICENSE`, `NOTICE` and `ATTRIBUTION.md` at `db0b8dd35` have
blob prefixes `6f461b24eea3`, `71d968f79b68` and `6c9e35da60eb` respectively.
The matching pinned `solidcore/` tree IDs at `e151d5ea` and `db0b8dd35`
verify retention of the nested tree under CD-7. The root documentation
does not replace these files or establish package contents.

The same sponsor banner appears in the vendored tree and is linked by its
README. The attribution does not establish that Blacksmith's original
permission covers IntCAD's public repository, source archives, or the
vendored placement. Confirm the scope with the rights holder; if it cannot
be documented, get an owner disposition rather than changing the frozen
tree. The asset guide's macOS installation description is not packaging
evidence or licence clearance. Where upstream brand terms and fork
attribution differ, record the earlier published version and terms relied
on, or obtain written permission for the intended use.

## K15: local enforcement is not the upstream path-count gate

At `db0b8dd35`, `scripts/ci/main_merge.py` selects `xtask layers` in the
repository lane. `xtask/src/main.rs::cmd_layers` builds the graph from
Cargo metadata and calls `layers::check`; this is the local graph check.
The candidate lacks root `xtask/solidcore-seam.txt`,
`xtask/solidcore-public.txt`, and `xtask/solidcore-paths.txt`. Frozen upstream
uses `xtask/src/solidcore_paths.rs` for the path-count limit (`91695f748`).
Do not mark the upstream check adopted because the local command has the
same name. Adoption of a path-level seam contract and the corresponding
fork-specific guide need a separate scope decision and gate evidence.

Reproduce lineage with `git rev-parse <pin>:<path>`, the root notice delta
with `git diff 0fd92bea2 db0b8dd35 -- NOTICE ATTRIBUTION.md`, and sponsor
scope with the pinned `ATTRIBUTION.md` row and `git show --stat 66951921a`.
No runtime, packaging or licence check was run for this audit.
