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
| `set_parameter` | Same update dispatch and argument-alias behavior in scope. | E01: typed reply and input-union evidence. |
| `check_parameters` | Existing trial/restore path is the preservation target; a restore can fail. | M05: body filter and summary options; do not treat the trial edit as read-only. |
| `undo` | Same previous-snapshot action, label reply, and empty-stack error path in scope. | Candidate runtime evidence only. |
| `redo` | Same next-snapshot action, label reply, and empty-stack error path in scope. | Candidate runtime evidence only. |
| `new_design` | Same replacement behavior, `Untitled` default, and `{name}` fields in scope. | Candidate runtime evidence for the typed engine reply and replacement effects. |
| `open` | Existing MCP fields and `doc.open` dispatch retained. | M06: new format/result behavior; CD-1/CD-5 boundaries remain as approved. |
| `save` | Existing path/format defaults and save dispatch retained. | E02: classify conditional reply metadata; CD-1 round-trip gate remains separate. |
| `list_commands` | Built-in query/filter/paging logic in scope. | Existing CD-5 plugin exclusion applies; E03: verify caller-visible built-in catalog contract. |
| `describe_command` | Built-in lookup and descriptor envelope in scope. | Existing CD-5 plugin exclusion applies; E03: descriptor contents can change with commands. |
| `inspect_design` | Base document inspection remains available. | M07: additional selection and response modes; M08: default sketch summaries only in server-wide compact mode. |
| `measure` | MCP option and dispatch layer in scope. | M09: changed measurement coverage and result behavior. |
| `body_topology` | Wrapper/cylinder grouping in scope. | Geometry and runtime parity are not established. |
| `crash_reports` | List/detail contract in scope; preserve existing Assistant restrictions. | Candidate runtime and authorization evidence only. |
| `running` | Comparable status payload in scope. | M10: the bridged stdio path can now return status during an in-flight call. |
| `pending` | Legacy result path comparable in scope; preserve guarded result handling. | M11: additional fields on recognized uncertified guarded completion. |
| `skill_reference` | A/C retain the reader and bundle contract. | B-only bundled material not adopted by this record; keep the current bundle and verify it at runtime. |

## Owner-pending decisions

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

E01–E03 require engineering evidence first: compare accepted input and reply fields for `set_parameter`; separate `save`'s conditional metadata from CD-1 container behavior; compare built-in catalog/descriptor contracts without treating a changed command catalog as a new MCP permission. If evidence establishes a semantic, effect, or permission change, record it as a separate owner decision rather than marking it equivalent.

## Verification checklist

- [ ] Independent review of the decision classifications against the pinned source audit.
- [ ] Record owner dispositions item by item; no approval inferred from C, this document, or an unrelated receipt.
- [ ] Run applicable candidate and landed-SHA checks before acceptance. Preserve #185's existing CI and strict reference gates.

The separate request to publish the Union report remains pending. It concerns a public example only and is not authorization to publish private artifacts or change the reference contract.
