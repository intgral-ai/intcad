# INT-1454 temporary union operand-order fallback

Status: approved narrow workaround; source-only. This plan and its tests are the removal record.

## Provenance

- Source base: `feat/int1454-ui` `c5f2f42fe6f06ed5c2c4ec1df04f7e9c6aae50ad`
  (diagnosis head `61835bfafb151c7d6803aeab195e824b6d8458d3`, one commit earlier; the
  relevant boolean path is byte-identical).
- Vendored solidcore: snapshot `524be9e1…`, upstream target `v0.44.1`, tree `e151d5ea6`.
  The vendored tree stays byte-identical. Do not edit `solidcore/`.
- Evidence: `/tmp/opencode/int1454-front-triangle-diagnosis/` (`ROOT-CAUSE.md`, `FACTS.md`,
  `REPRO.md`, exported operands and hash manifest).

## Defect

`regressions/front_triangle_hull_triangle`, entity `FrontTriangle/F15:start`, exact face
point `[499.91353034098677, -3.886305195470338, 518.9631745038374]`, distance `5.0`,
`solid.extrude` with `operation: "join"` (matrix 2) and `solid.press_pull` (matrix 8)
fail with:

```text
SMLib boolean: operation: solidcore: SmApiBoolean: topology::proto_topology::p1: the proto edge is not listed
```

The same valid, closed operands union successfully when SMLib sees them in the reverse
order, with the additive volume of the two operands (contact-only). This is a vendored
solidcore operand-order asymmetry, not a face lookup or IntCAD adapter error.

## Approved boundary

- IntCAD kernel only: `crates/kernel/src/ops.rs` and `crates/kernel/src/history.rs`;
  tests in the existing `crates/engine/src/boolean_regression_tests.rs`; this plan file.
- Do not edit vendored `solidcore/`, `crates/doc/src/eval.rs`, shared manifests/lockfiles,
  baselines, beta exceptions, or another worker's validation/Mastra files.
- Ordinary (`original`) union is always tried first, with the original external operand
  order `[a, b]`.
- One retry only, only for `BoolOp::Union`, only when the ordinary call failed with the
  exact fingerprint above, and only when both operands are B-rep bodies with exactly one
  material region each. This avoids hiding a `union_by_lumps` refusal.
- The retry calls SMLib in `[b, a]` order, then permutes every history operand `0 <-> 1`
  (`FaceOrigin`, `EdgeOrigin::Edge`, and both sides of `EdgeOrigin::Meet` via the canonical
  `Side::map_operand`/`EdgeOrigin::meet`). The result history is attached over the original
  `[a, b]` operands and checked by `Body::with_history`.
- Fail closed: any retry failure, empty result, missing history, remap failure, or history
  check failure returns the **original** error. No result is accepted with guessed or
  unverified history. The engine regression must also prove exact face names; if lineage
  naming is not exact, stop and report instead of accepting the fallback.
- Undo, replay, save/load, body name, material and validation semantics remain the ordinary
  join semantics: the result is the ordinary union body with exact histories over `[a, b]`.

## Checklist

- [x] Verify base, provenance and exact defect fingerprint read-only.
- [x] Write this plan/removal record.
- [ ] Commit RED product tests on `fix/int1454-union-fallback`; push; observe the exact
  failure in remote CI (no local build).
- [ ] Implement original-first retry + exact fingerprint + single-material-region guard +
  history `0 <-> 1` permutation in the kernel adapter only.
- [ ] Add the independent original-path audit and the retry-guard/history unit tests.
- [ ] Push GREEN; remote CI must pass the targeted engine tests and repository checks.
- [ ] Report RED/GREEN run IDs, HEADs, dirty state, commands/exits and any NOT RUN checks.

## Invariants and tests

- The result is one closed `FrontTriangle` body with the additive volume of target + tool
  (contact-only union), not a tolerance allowance.
- `History` has two operands and checks against the original `[a, b]` bodies; every face
  name is exact (`solidcraft_doc::naming::face_name_exact`). A target face away from the
  contact keeps its pre-join name; a point on the tool's far cap carries the join feature's
  role name.
- A dependent `timeline.edit` of the distance inside the failing window re-evaluates with
  the same exact history/naming; `edit.undo`/`edit.redo`, full script replay, and a saved
  document round-trip preserve volume, validity and names.
- Independent original-path audit calls the unpatched ordinary kernel entry. It diagnoses
  `still fails -> fallback needed` or `now succeeds -> remove fallback`; it does not assert
  that the upstream bug is a permanent product contract.
- Guard tests: original first; retry only on the exact gate; other errors/operations no
  retry; failed retry returns the original error and changes no model state.

## Removal / expiry

On the next solidcore/upstream upgrade, disable the fallback and run the original-order
regression first.

- If the original path passes all geometry/history/naming tests: delete the fallback and its
  audit seam, keep the product regression tests (now exercising the ordinary path).
- If it still fails: revalidate the fingerprint and geometry from scratch; do not extend the
  workaround silently.
- If the algorithm, error text or naming changes: suspend the fallback and re-review. Never
  auto-extend it.

Temporary callsite: `crates/kernel/src/ops.rs` (`boolean`, `boolean_original_order`,
`retry_swapped`, `first_then_retry`) and the history permutation in
`crates/kernel/src/history.rs`. The approach is intentionally removable as one branch plus
one helper.
