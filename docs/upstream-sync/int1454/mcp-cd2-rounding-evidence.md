# CD-2 E01 follow-up: rounding and remainder after unit conversion

Scope: baseline `0fd92bea266e7dcea5ce09676ed9ff9ae6c66d7f` (A), frozen upstream `e151d5ea6866fb055e011379153fb1cae0566f22` (B), candidate `db0b8dd350bbdffed8927845c86c5028d280cb0e` (C). This supplements [E01](mcp-cd2-decisions.md#e01e03-source-findings); it does not extend M12–M14, request approval, or change a gate.

## Source evidence

B and C have identical `crates/doc/src/expr.rs` blob `df45e6bab3676d905c95ba8340d14ea802322682`. The two rules trace to upstream `c29f396f72996c9e45359aff454237ea5146dfb4` (rounding, #12489) and `81d39a7331fdd76802201b099264764702fdba04` (remainder, #16490). The candidate contains the same blob via snapshot commit `7cebdfa281f10e655a0e636097695ed272c606be`.

- **Rounding functions.** In A, `floor`, `ceil`, `round`, and `trunc` apply the respective operation directly to the value scaled into its default unit (`crates/doc/src/expr.rs:979–991`). C first snaps a value to its nearest integer within `min(4 × EPSILON × abs(nearest), 0.125)` in that scale, then applies the selected operation (`expr.rs:1033–1050`). This is a shared code-path change, not evidence that every function returns a different result.
- **Euclidean remainder.** A uses `rem_euclid` directly after unit conversion and the zero-divisor check (`expr.rs:890–900`). C returns a zero remainder with the original dimensions when the converted quotient is near a nonzero integer, `abs(nearest) < 1e12`, and the gap is at most `4 × EPSILON × abs(nearest)`. Otherwise it retains `rem_euclid` (`expr.rs:935–953`). The quotient check is a distinct rule from rounding-function snapping.
- **MCP boundary.** C's `set_parameter` resolves the input alias and forwards the expression to `parameters.change` (`crates/mcp/src/tools.rs:1180–1184`); the changed operations are in the shared expression engine. There is no new MCP field or permission in these changes. Do not preserve old results with a second evaluator: any baseline-retention option needs a compatible shared-engine disposition and evidence.
- **Read-only witness.** Existing MCP `execute` can invoke `expr.evaluate` on an open document. It uses the same evaluator without adding a parameter or editing geometry. Check `value` and `display_value` after reply formatting, separately from the raw evaluator: C calls `round_reported` for each field (`crates/engine/src/cmd/parameters.rs:638–700`). Exact tolerance checks belong at the Doc level, where reply formatting cannot hide differences.

## Independent arithmetic check

Both revisions use `25.4` millimetres per inch. A Python binary64 calculation of the relevant branches gave a quotient of `2.9999999999999996`, a nearest-integer gap of `4.4408920985006262e-16`, and a tolerance of `2.6645352591003757e-15`.

| Input | A branch prediction | C branch prediction |
| --- | --- | --- |
| `floor(3 in / 1 in)` | `2` (unitless) | `3` (unitless) |
| `3 in % 1 in` | `25.399999999999991 mm` | `0 mm` |

These are arithmetic predictions, not executions of either product revision.

For the read-only witness with `unit:"in"`, the remainder case predicts baseline reply fields `value:25.4` (mm) and `display_value:1` (inches), versus zero in both fields on C. Useful controls are `ceil(3 in / 1 in)` (3 on both) and `mod(2.5 in, 1 in)` (0.5 inches on both). The remainder threshold is on the rounded quotient, not the result in millimetres. Rounding uses the contextual default unit for simple lengths and angles, not necessarily the expression's written suffix; its tolerance also applies to literals inside the snap window.

## Retained test evidence

Both historical Doc JUnits report passes for `expr::tests::rounding_functions_ignore_float_noise` and `expr::tests::mod_ignores_float_noise_in_the_quotient`. Their `expr.rs` blob matches B/C. These are historical evaluator tests, not current-candidate or end-to-end results.

- [Run 44505747549](https://github.com/intgral-ai/intcad/actions/runs/44505747549), `c5f2f42fe` tree: `rust/junit.xml` SHA-256 `ebd54d6fca6657697b96c3507ea82dca6aa4e81c48f9c8d9acbf65e1b5bfe925`.
- [Run 44789018375](https://github.com/intgral-ai/intcad/actions/runs/44789018375), `c14f2a1b0`: `rust/junit.xml` SHA-256 `ba28f24fc02bea08569c3f0265b1cf92ac148b8583c9578a896c753328aadd45`.

No MCP or saved-project receipt is claimed here.

## Review and disposition

- [x] Verify the A/C branch differences and arithmetic witnesses.
- [x] Confirm B provenance and recover existing historical Doc execution evidence.
- [ ] Obtain independent review of source scope, numerical witnesses and receipt attribution.
- [ ] Evaluate the two observable changes separately for CD-2. Verify each discriminator and unchanged control through MCP, the Doc-level threshold and remainder fallback, and persisted parameter recomputation before integration; no new allowances or weakened assertions.
