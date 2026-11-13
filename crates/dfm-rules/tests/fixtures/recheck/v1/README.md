# Recheck fixtures (report 1.8)

Two INTdfm-shaped reports and the `recheck` object `recheck.compare(earlier, later)` made from
them. The Rust test compares the whole `Recheck` — marks, guard flags, `not_compared`, labels,
`moved_mm` and every message — and then compares the canonical report bytes with the `recheck`
embedded.

**Every pair here is TEST-ONLY.** No OCP is installed in the environment this port was written in,
so a real before/after STEP pair could not be recorded with `replay.record`. Instead:

* `earlier` is the report of a committed schema-2 recording (`tests/fixtures/dfm-replay/v1`);
* `later` is what the *real* INTdfm rule and report layer (`replay.assemble`) produces for the same
  input with one measurement or declaration edited — a hole moved, a hole lost, a second hole
  added, a table or tool version changed;
* `expected` is `recheck.compare(earlier, later)`, never written by hand.

That edits facts rather than re-reading a STEP file, so it cannot prove the App measures the part
the same way; it does prove the comparison layer reproduces Python for the reports it is given.
When OCP is available, record a real pair and add it here unchanged, keeping the synthetic ones:
they reach paths one part pair cannot (a structurally ambiguous feature, a changed tool version).

| Case | What it pins |
|---|---|
| `synthetic.same_part` | no change: every marked finding `still_there`, no flags |
| `synthetic.two_holes_hole_moved` | a two-hole part with one hole moved 0.2 mm |
| `synthetic.row_hole_moved` | a row of holes; the moved hole's review follows, `moved_mm = 0.2` |
| `synthetic.feature_cannot_follow` | a hole moved past the 5t reach: `cant_match`, not `fixed` |
| `synthetic.ambiguous_feature` | two later holes within 0.05 mm: no feature is picked |
| `synthetic.lost_hole` | `HOLE_COUNT_DROPPED`, and the lost hole's finding is `cant_match` |
| `synthetic.new_edge_review` | a review that was not there before: `new` |
| `synthetic.fixed_review` | a review gone with the hole still there: `fixed` with the public-value note |
| `synthetic.profile_changed` | same shop name, another file: `SHOP_NUMBERS_CHANGED` |
| `synthetic.tool_changed` | a different rule-layer version: `TOOL_CHANGED` |
| `synthetic.declared_changed` | another declared material: `DECLARED_CHANGED` |

Do not edit a fixture by hand. Regenerate the pairs from the INTdfm checkout with the generator
used for this batch (`PYTHONPATH=src`, no OCP), or record real ones with `replay.record`.
