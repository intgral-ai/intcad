# INT-1454 CD-2 decision register

Scope: IntCAD baseline `0fd92bea266e7dcea5ce09676ed9ff9ae6c66d7f` (A), upstream `e151d5ea6866fb055e011379153fb1cae0566f22` (B), candidate `db0b8dd350bbdffed8927845c86c5028d280cb0e` (C). Source basis: [reviewed contract audit at `8af1ab8aa`](mcp-cd2-contract-audit.md). This register does not approve a change, replace the runtime gate, or reopen [`version`, `cancel`, or `batch.path`](mcp-tool-map.md#candidate-a1-disposition).

**Equivalent in scope** means the named source behavior matches, not that all replies, backend results, or deployed authorization match. **Evidence pending** is an engineering action, not an approval request. **Owner pending** means retaining the identified change requires a separate decision under CD-2; otherwise preserve the baseline contract or leave the item unintegrated. No permission expansion is proposed. A stable `Effect` annotation is not proof of authorization.

## Per-tool disposition

| Tool | Source-equivalent scope | Unresolved disposition |
| --- | --- | --- |
| `execute` | Base dispatch and IntCAD document guard retained. | M01: compact result fields. |
| `batch` | Inline order/defaults and IntCAD guard retained. | M02: compact step fields; M03: early-rejection rollback flag. |
| `screenshot` | Capture selection/fallback and PNG/metadata contract in the inspected path. | Candidate runtime evidence only; preserve file scope and existing capture effects. |
| `compare_picture` | Scoped reference read and overlay/metrics behavior as shipped. | Do not enable the documented but unforwarded `part_fills_crop`. Runtime evidence pending. |
| `export` | Existing path-based dispatch and file scope are the preservation target. | M04: expanded explicit format enum; infer no new backend support from the enum. |
| `set_parameter` | Same update dispatch, argument-alias behavior and default reply shape in scope. | E01: M12–M14 cover three observed changes. Other expression changes remain open. |
| `check_parameters` | Existing trial/restore path is the preservation target; a restore can fail. | M05: body filter and summary options; do not treat the trial edit as read-only. |
| `undo` | Same previous-snapshot action, label reply, and empty-stack error path in scope. | Candidate runtime evidence only. |
| `redo` | Same next-snapshot action, label reply, and empty-stack error path in scope. | Candidate runtime evidence only. |
| `new_design` | Same replacement behavior, `Untitled` default, and `{name}` fields in scope. | Candidate runtime evidence for the typed engine reply and replacement effects. |
| `open` | Existing MCP fields and `doc.open` dispatch retained. | M06: new format/result behavior; CD-1/CD-5 boundaries remain as approved. |
| `save` | Existing path/format defaults, save dispatch and ordinary schema-valid reply retained. | E02: M15 covers conditional newer-file reply fields only; CD-1 round-trip gate remains separate. |
| `list_commands` | Built-in query/filter/paging logic and default reply envelope match in scope. | E03: candidate runtime evidence pending. Existing CD-5 plugin exclusion applies; catalog contents are not frozen. |
| `describe_command` | Built-in descriptor envelope and case-insensitive lookup retained. | E03: M16 covers suggestion eligibility and ranking only. SEC-1 command-ID normalization and CD-5 plugin exclusion are not reopened. |
| `inspect_design` | Base document inspection remains available. | M07: additional selection and response modes; M08: default sketch summaries only in server-wide compact mode. |
| `measure` | MCP option and dispatch layer in scope. | M09: changed measurement coverage and result behavior. |
| `body_topology` | Wrapper/cylinder grouping in scope. | Geometry and runtime parity are not established. |
| `crash_reports` | List/detail contract in scope; preserve existing Assistant restrictions. | Candidate runtime and authorization evidence only. |
| `running` | Comparable status payload in scope. | M10: the bridged stdio path can now return status during an in-flight call. |
| `pending` | Legacy result path comparable in scope; preserve guarded result handling. | M11: additional fields on recognized uncertified guarded completion. |
| `skill_reference` | A/C retain the reader and bundle contract. | B-only bundled material not adopted by this record; keep the current bundle and verify it at runtime. |

## Owner disposition

The [owner accepted the proposed dispositions](https://linear.app/intgral/issue/INT-1454#comment-b67c8020-b61e-4ad7-8261-d48a8a6eab10):

- Preserve the IntCAD baseline: M01, M02, M03, M06 (within CD-1 and CD-5), M08, M11.
- Adopt only after each item's acceptance tests pass: M15, M16.
- Defer this cycle: M04, M05, M07, M09, M10, M12, M13, M14, M17, M18. M17 and M18 remain separate shared-engine decisions.

The [owner deferred M12 and M13](https://linear.app/intgral/issue/INT-1454#comment-8f0a864b-66d6-4c17-bfb4-35ff6467893b) after compatibility testing. With a legacy `μm = 2 mm` parameter, `μm * 3` is 6 mm and `250 μm` is 500 mm (test `95d49274`). A saved `(5 ft 3 in) / (1 mm * 1 mm)` evaluates to 116128.8 after reopening (test `3d6bc849`). Preserve the existing parser and do not normalize these inputs through MCP this cycle. Use `µm` or `um`, and `5 ft + 3 in`, to express the units explicitly.

This records the decision, not implementation or acceptance evidence. No legacy aliases, broader permissions, or persistence-policy changes are approved.

## Decision inputs and acceptance gates

Each ID is independently decidable. The evidence below is required for the selected disposition; it is not a test run or a waiver.

| ID | Change to approve or retain baseline | Evidence required before integration |
| --- | --- | --- |
| M01 | Retain C's per-call `execute verbose:false` field changes, or preserve A's compact reply fields. | Compare exact fields for representative compact and normal replies in headless and bridged calls; verify unchanged guard contract. |
| M02 | Retain C's per-call `batch verbose:false` step-result changes, or preserve A's fields. | Compare per-step fields for success and failure independent of M03; verify the guarded envelope. |
| M03 | Retain C's `rolled_back` value on early step-mapping rejection, or preserve A's `false`. | Verify both rollback settings, `undone:0`, no step execution, and interpretation by consumers of the flag. |
| M04 | Accept explicit `stp`, `iges`/`igs`, and `glb` in the MCP enum, or retain A's explicit enum. | Separate enum changes from extension inference; verify supported outputs, artifact set and unchanged file scope. |
| M05 | Retain `check_parameters` body filtering and summary options, or preserve A's option set. | Verify defaults and projected reports plus restoration behavior; do not expand permissions for trial edits. |
| M06 | Accept changed `open` format/result behavior within approved CD-1 and CD-5, or preserve the baseline MCP-visible behavior where compatible with those decisions. | Identify each accepted format and reply variant, verify import and document replacement; do not reopen format 58, newer-file preference or plugin exclusions here. |
| M07 | Retain `inspect_design` timeline/body selection and additional detail modes, or keep A's selection contract. | Compare omitted and explicit options; distinguish standalone enrichment from inline and resource replies. |
| M08 | Retain opt-in server-wide compact serialization and return summaries for omitted `sketches` only when that mode is enabled, or preserve A's text/default contract. | Compare JSON content separately from text formatting and verify omitted/explicit sketches with compact mode off and on; independent of M01/M02. |
| M09 | Accept added measurement coverage and closest-point result behavior, or preserve the baseline MCP-visible measurement contract. | Compare representative solids, 3D wires and exact/approximate results against the approved geometry contract. |
| M10 | Retain concurrent `running` status in the bridged stdio path, or preserve A's serial behavior on that path. | Verify ordering and status during an ordinary in-flight bridged stdio call, lifecycle and unchanged authenticated scope. |
| M11 | Retain additional guarded `pending` completion facts, or preserve A's guarded reply shape. | Compare known and uncertified completion replies; verify no retry, redispatch, permission change, or reinterpretation of an unknown outcome. |
| M12 | Accept Greek-letter `μm` as a micrometre spelling through `set_parameter`, or leave the change unintegrated. | Compare accepted spelling variants and evaluated values. |
| M13 | Interpret mixed feet-and-inches notation as a single length through `set_parameter`, or leave the change unintegrated. | Compare an example such as `5 ft 3 in` and verify the evaluated length and reply. |
| M14 | Accept a negative update to a supported sided sketch dimension that flips its side and normalizes the expression, or leave the change unintegrated. | Verify the actual sketch change and normalized reply. Separately verify that updating intrinsic sizes to negative values is refused through MCP `set_parameter`; the cited creation checks do not cover updates. |
| M15 | Return conditional `kept_format`, `warnings`, and `not_kept` on newer-file MCP saves, or preserve the baseline MCP reply shape without changing CD-1 behavior. | Verify exact fields for ordinary saves and newer-file saves with and without reported loss in both headless and bridged MCP. Reply metadata is not proof of lossless persistence. |
| M16 | Retain the new eligibility and ranking of same-namespace alternative verbs in unknown `describe_command` suggestions, or preserve baseline suggestions for this read-only path. | Verify which suggestions appear, their ordering, and the error outcome in standalone and inline calls. Suggestions must not act as execution aliases. |

## E01–E03 source findings

- **E01:** The input type array in A and `anyOf` in B/C accept the same number/string types in scope; dispatch resolves the same argument aliases. The typed engine reply retains the ordinary fields; Warn-mode metadata is separate from the default reply. M12–M14 are not an exhaustive expression comparison. Unit-conversion rounding affects results on the same expression path, including `floor(3 in / 1 in)` and `3 in % 1 in`; these deltas remain unresolved and are not covered by M12–M14. Candidate sources: `crates/mcp/src/tools.rs:239–257,1180–1184`; `crates/engine/src/cmd/parameters.rs:158–294`; `crates/doc/src/expr.rs:184–200,935–953,1033–1050,1510,1747–1757,1834–1840,1866–1888`; `crates/engine/src/cmd/sketch_constraints_tests.rs:1984–2002`. These test sources are not runtime receipts.
- **E02:** A returns `path`, `bytes`, `name`; ordinary schema-valid C saves retain those fields. B/C add `kept_format` on newer-file saves and emit `warnings` and `not_kept` only when something could not be kept. The file write precedes the reply. C standalone and inline MCP forward only `path` and `format`. Candidate sources: `crates/mcp/src/tools.rs:1196,1465`; `crates/engine/src/cmd/file.rs:1704–1734`; `crates/engine/src/schema/save.rs:144–168`. The existing MCP encoding test (`crates/mcp/src/tests.rs:886–914`) does not assert the save reply.
- **E03:** Default built-in query/paging and descriptor envelopes match in scope; changing command schema contents is not itself an MCP permission change. C accepts surrounding whitespace in command lookup; this was already present in merged SEC-1 `96a247ccb`, including the descriptor's use of `find_command`, and is not a new item here. C can admit suggestions that A would not have offered and change their ordering; M16 covers both. Candidate sources: `crates/mcp/src/tools.rs:1126–1135,1204–1296,1429–1438`; `crates/engine/src/cmd/inspect.rs:793–810`; `crates/engine/src/cmd/mod.rs:464–522`. Server-wide compact formatting remains M08; plugins remain excluded under CD-5.

The E01–E03 comparison is source evidence only. See the owner dispositions above; the comparison does not establish acceptance on an integrated SHA.

## Verification checklist

- [ ] Independent review of the decision classifications against the pinned source audit.
- [ ] Record owner dispositions item by item; no approval inferred from C, this document, or an unrelated receipt.
- [ ] Run applicable candidate and landed-SHA checks before acceptance. Preserve #185's existing CI and strict reference gates.

The separate request to publish the Union report remains pending. It concerns a public example only and is not authorization to publish private artifacts or change the reference contract.
