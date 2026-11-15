# INTdfm report-parity fixtures (schema 2)

One folder per recording in INTdfm's `tests/fixtures/dfm-replay/v1`: the rule-layer input
`build_report` received and the canonical bytes of the report it produced. These are **report**
fixtures: the findings are the Python rule layer's, so a difference means `intdfm_report.rs`
disagrees with `report.py`, never that a Rust rule disagrees.

| Part of the file | Where it comes from |
|---|---|
| `source` | the recording's `source` (commit, rule-layer hash, report version) |
| `case` | the recording's `case`, minus any STEP bytes or absolute path |
| `input.findings` | the **pre-merge** findings. The recording stores only the merged report, so these are captured by running `replay.assemble(recording["input"])` with a spy on `report.build_report` |
| `input.declared`, `file_sha256`, `core_version`, `ocp_version`, `occt_package`, `profile`, `snapshot_id`, `project_file`, `reader_log`, `critical` | the recording's `input.declared`/`input.critical` and `input.traceability` |
| `input.header` | `rules.header(findings, shop)`, computed from the **pre-merge** findings |
| `input.bends`, `input.part` | `rules.bends_of(solid)`, `rules.part_of(solid)` (unrounded; the report rounds them) |
| `input.shop_layer` | the shop profile's layer, `null` without a shop, so the test can also check `header()` |
| `expected.header` | the recorded report's `header` |
| `<case>.canonical.json` | `report.canonical_json(recording["expected"]["report"])`, byte for byte |

`synthetic.id-collision` is the one case with no recording. It repeats a flat finding with another
message so two findings share a base id, which pins the `-2` suffix and the tie-break order. Its
expected bytes still come from `report.build_report`; the case is marked `TEST-ONLY`.

## Regenerating

Run from an INTdfm checkout with `PYTHONPATH=src`, no OCP needed:

```python
import json
from intdfm import replay, report, rules

captured = []
original = report.build_report
def spy(findings, **kwargs):
    captured.append(list(findings))
    return original(findings, **kwargs)
report.build_report = replay.report.build_report = spy
actual = replay.assemble(recording["input"])          # raises if expected.report differs
pre = captured[-1]
header = rules.header(pre, shop_or_none)              # must equal actual["header"]
```

Write `input.findings = pre`, the metadata above, and
`<case>.canonical.json = report.canonical_json(recording["expected"]["report"])`. Do not edit a
fixture by hand, and do not widen a tolerance: a changed recording means re-record it in INTdfm
first. The three cases `hole_to_bend.u_holes`, `hole_to_edge.opening` and
`hole_to_edge.plate_holes` were untracked in the INTdfm checkout at generation time; re-run the
generator if they are recorded again.
