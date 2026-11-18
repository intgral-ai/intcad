# Sheet-metal DFM

## Sub-features

Run a sheet-metal check and read the persisted report for the checked design and project profile.

## How to get to it (user POV)

Open a saved sheet-metal design in a project with a shop profile. Run the check from
Assistant for an Assistant change, or use the public command interface for the worker and rule.

## Driving it with the existing DFM harness

Use a case from the [catalogue acceptance guide](../../../../docs/dfm-catalogue-acceptance.md).
For example, the `plate` case expects a 2 mm hole to fail the 3 mm minimum:

```sh
SOLIDCRAFT_DFM_SAMPLE=plate cargo test -p solidcraft-engine --test dfm_acceptance catalogue_samples
```

For freshness, run `a_changed_declaration_makes_the_old_report_stale_and_the_next_check_decides`
in the same test target. In the app, use `dfm.check` and read the completed run with `inspect.dfm`.
Compare the run ID, target rule state and measurement with the saved report and findings.
For Assistant card changes, run the [browser journeys](assistant.md#check-assistant-decisions-and-dfm-findings-in-chrome)
to exercise finding details and selection messages. For the native Review rail, the existing
`cargo test -p solidcraft-ui-egui --lib dfm_review_tests` checks picking a finding and dragging
a report onto or outside the Assistant.
For a card or selection change, capture the card, click the affected finding and observe the
expected model highlight. See the [desktop evidence](../../../../docs/dfm-acceptance-report.md#6-desktop-evidence-l5).

## Gotchas

An unsupported rule or missing threshold must remain visible. After editing a declaration,
check both the old run's stale state and the new run's result. The catalogue test covers the
worker and storage. A card or selection change requires desktop evidence.
