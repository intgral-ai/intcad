---
name: solidcraft-ground-rules
description: "Use before adding any dependency, icon, image, font, test reference data or product wording to SolidCraft, or when unsure whether something may be copied, read or named. Covers independence from other CAD products, GPL code, asset attribution and the product name."
---

# Independence, licences, assets and the name

Hard rules; reasons: CONTRIBUTING.md, "Rules that aren't negotiable".

## Independent
- Design from public documentation, CAD conventions, user feedback and public format specs. Tests use analytic values (`tools/oracle-analytic/`) or open-source tools.
- No other-product code, icons, artwork, text or templates; only the scoped grant below permits reuse; no CAD/UI assets.
- Never drive, script, automate or extract commercial CAD data (Fusion, SolidWorks, Onshape): no test values, measurements, command lists or generated files.
- Our command ids: dotted, lower-case `<area>.<thing>[.<variant>]`, never another product's.

## desktop-only Mastra distribution exception
- Rights gate: `docs/mastra-distribution.md` and `packaging/mastra/source-records.json`: the intgral-ai grant at `acb5048120b53ae1b4502724b70d1283e8c9daf4` covers only selected owned Vector Studio orchestration/agent-loop/model-runtime code, under `MIT OR Apache-2.0`, after per-file ownership, licence, notice, dependency and third-party review. Third-party content keeps its licences/notices; `ee/` and excluded roles are forbidden.
- 3 records `needs-human`, 2 `rejected`, approvals empty; no source is importable yet and evidence never grants permission. unverified evidence is not permission: rights review and public redistribution remain human decisions.
- Only opt-in desktop `packages/mastra-runtime/` permits the TS companion and pinned Node; host/core/application stay Rust/wasm, no Node.

## No GPL, LGPL or AGPL code
- Never read, copy or link GPL/LGPL/AGPL code: OpenCASCADE, FreeCAD (planegcs too), SolveSpace, CadQuery/OCP, LibreCAD or QCAD.
- Sole exception, desktop web view: unmodified system GTK 3/WebKitGTK 4.1, dynamically linked by `wry` for the Assistant (`docs/web-view-licences.md`); never copied, vendored, static-linked or modified; notices ship; legal review open; never a CAD kernel, solver, geometry or UI library.
- `tools/freecad-oracle/` computes reference values through FreeCAD's public scripting API, separately; nothing linked, vendored or shipped. Solver/blends are ours.
- Explain permissive dependencies in the PR. `cargo xtask licences --check` inventories Cargo, not npm/Node or file rights. Never edit generated `THIRD-PARTY-LICENSES.txt` in a PR. [storytold](https://github.com/storytold) projects may guide conventions.

## Assets: the most serious rule
- No commercial CAD imagery, fonts or artwork; never commit its screenshots. Every asset is original/openly licensed with an ATTRIBUTION.md row (`cargo xtask assets`).
- Original SVG masters: `assets/icons/ui/design/svg/`; compilation: `crates/ui-egui/src/icon_svg.rs`.
- Before icon or media edits, read `assets/AGENTS.md` and its icon guides; use canonical bases and finished references; `cargo xtask icons` validates.

## The name
- Displayed name: **Intcad**. Internal identifiers stay `solidcraft`: crates, binaries, `.sc`, `solidcraft/N`, `SOLIDCRAFT_*`, settings, app IDs and MCP.
- Compatibility: `docs/compatibility.md`; no new internal `solidcraft` names without a reason. Upstream keeps the SolidCraft name/logo; credit it only as upstream.
- Terms: `docs/book/src/reference/glossary.md`; units mm/radians, Z up.
