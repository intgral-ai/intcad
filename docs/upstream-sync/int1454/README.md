# INT-1454 — upstream sync to SolidCraft v0.44.1

Phase 0 record for the module-level sync approved in the INT-1454 Spec. Nothing here claims the
sync is implemented; it records what the repository and the upstream tag actually contain, how the
work is sliced, and what still needs a human decision.

| Document | Contents |
| --- | --- |
| [baseline.md](baseline.md) | frozen SHAs, release windows, diff scale, CI/test baseline, machine limits |
| [ledger.md](ledger.md) | upstream-sync ledger: units, commit ranges, function, IntCAD status, licence, dependencies, disposition |
| [dag.md](dag.md) | real crate dependency graph, conflict matrix, ownership, change waves |
| [licence-audit.md](licence-audit.md) | licence map, brand boundary at v0.44.1, adoption rules |
| [test-matrix.md](test-matrix.md) | PR-level and final gates, platform matrix, E2E protocol |
| [pr-plan.md](pr-plan.md) | PR slicing, streams, order, rollback |
| [risks.md](risks.md) | risk register, blockers, decision candidates |
| [security-findings.md](security-findings.md) | verified security issues in today's tree and the upstream fixes that address them |
| [generated/commits.tsv](generated/commits.tsv) | every non-merge upstream commit with area labels |

Frozen references: upstream **v0.44.1** = `e151d5ea6866fb055e011379153fb1cae0566f22`,
IntCAD **develop** = `0fd92bea266e7dcea5ce09676ed9ff9ae6c66d7f`, fork point
`06749ac4`. Range covered: 4,994 commits (4,939 non-merge).

Phase 0 does not authorise the migration yet: the ledger needs an independent review of its
`Already Covered` / `Not Applicable` rows and the owner's decisions on CD-1 to CD-4.
