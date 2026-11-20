# INT-1454 — upstream sync to SolidCraft v0.44.1

Phase 0 record for the module-level sync approved in the INT-1454 Spec. Nothing here claims the
sync is implemented; it records what the repository and the upstream tag actually contain, how the
work is sliced, and what still needs a human decision.

| Document | Contents |
| --- | --- |
| [baseline.md](baseline.md) | frozen SHAs, release windows, diff scale, CI/test baseline, machine limits |
| [ledger.md](ledger.md) | upstream-sync ledger: units, commit ranges, function, IntCAD status, licence, dependencies, disposition |
| [blocked-dispositions.md](blocked-dispositions.md) | itemized exclusions, pending decisions, and engineering gates; not sign-off |
| [dag.md](dag.md) | real crate dependency graph, conflict matrix, ownership, change waves |
| [licence-audit.md](licence-audit.md) | licence map, brand boundary at v0.44.1, adoption rules |
| [test-matrix.md](test-matrix.md) | PR-level and final gates, platform matrix, E2E protocol |
| [pr-plan.md](pr-plan.md) | PR slicing, streams, order, rollback |
| [risks.md](risks.md) | risk register, blockers, decision candidates |
| [security-findings.md](security-findings.md) | verified security issues in today's tree and the upstream fixes that address them |
| [review-brief.md](review-brief.md) | what an independent reviewer must check before the gate passes |
| [generated/commits.tsv](generated/commits.tsv) | every non-merge upstream commit with area labels |
| [tools/coverage.py](tools/coverage.py) | the coverage proof: assigns every commit to an area, exits non-zero on an orphan |

Frozen references: upstream **v0.44.1** = `e151d5ea6866fb055e011379153fb1cae0566f22`,
IntCAD **develop** = `0fd92bea266e7dcea5ce09676ed9ff9ae6c66d7f`, fork point
`06749ac4`. Range covered: 4,994 commits (4,939 non-merge).

Phase 0 passed independent review, and the owner approved CD-1…CD-7 on 2026-11-17 (ledger §7).

## Execution index

| Work | Branch / PR | State |
| --- | --- | --- |
| Phase 0 archive | `docs/int1454-phase0`, PR #177 | open, CI green |
| SEC-1 agent-gate bypass | `fix/int1454-agent-gate-aliases`, PR #179 | open, independent review approved |
| Rust 1.99.1 pin + CI toolchain | `build/int1454-toolchain-pin`, PR #180 | open, CI running |
| K0 kernel snapshot (vendored solidcore + kernel/smlib/geom/dynamics, `crates/step` removed) | `feat/int1454-kernel-snapshot` | WIP checkpoint pushed; workspace compiles; kernel tests running |

Next gates per the DAG: kernel rig (`cargo xtask kernel-cases --known-failing`), then doc/engine,
then MCP (CD-2 tool map first), then UI adaptation. Brand assets, plugin runtime and the colour
dedup stay out by CD-5/CD-6 and ledger U4.
