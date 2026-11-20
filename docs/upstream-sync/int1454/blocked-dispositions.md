# INT-1454 Blocked disposition worksheet

Scope: the ledger at integration candidate `db0b8dd350bbdffed8927845c86c5028d280cb0e`, the prior independent disposition review, and the acceptance receipts. This worksheet does not reclassify rows, grant a waiver, or sign off on the migration.

| Item | Current evidence and disposition | What remains |
| --- | --- | --- |
| A3 — Wasmtime plugin runtime | Blocked; CD-5 explicitly approves deferral. | A future product decision to ship plugins, dependency licence and packaging review, and capability security review. |
| U18 — plugin UI | Blocked by A3. CD-5 excludes the runtime from this sync; the ledger records U18 as a dependent exclusion. | Keep out of this sync. Record U18 separately in owner sign-off; a later implementation needs an approved runtime and UI acceptance. |
| U9 — `ui-kit`, `look`, `theme` | Blocked under approved CD-6. | Keep brand-era code out. A later proposal would need proven rights or an independently implemented equivalent. |
| U10 — post-brand icons and assets | Blocked under approved CD-6. | Keep excluded; use original or proven pre-brand assets for future work. |
| U11 — kit-coupled panel foundation | Blocked under approved CD-6; the upstream implementation depends on the excluded UI kit. | Any later adaptation must establish compliant provenance and run against IntCAD's UI without the excluded kit. |
| CD-6 — unnumbered chrome | Approved exclusion includes licence-uncertain chrome and `prefs*`. The prior review indexed paths and blob identities, not licence clearance. | Require a provenance decision before relying on uncertain code, including code already present in the candidate. |
| U14 — airfoil desktop entry | **Blocked — owner-approved deferral for this sync.** Retain `sketch.insert_airfoil` and existing Agent access. The selected case `a_naca_0012_section_extrudes_to_its_area_times_the_depth` passed in the otherwise red historical c5 native run. At `db0b8dd35`, the `AGENT_ONLY` entry is attributable to IntCAD commit `7755e4c47`, but its containing `workspace.rs` remains on the CD-6 exclusion list. The alternative drawing-import route is not independently cleared: proposed dialog helpers include U9 material. See the [owner disposition record](https://linear.app/intgral/issue/INT-1454#comment-ba9b9a09-bed6-40e4-829f-460b1f1ac241). | Before resuming, verify a compliant routing path under CD-6 and complete desktop acceptance. Backend evidence is not UI acceptance. |
| U17 — problem tickets | **Blocked — owner-approved deferral for this sync.** Do not connect to the upstream ticket service. See the [owner disposition record](https://linear.app/intgral/issue/INT-1454#comment-5b68d6cf-6587-49b1-bc37-a359986312c2). | Before resuming, define the IntCAD-owned intake channel and how reports will be handled. |
| U17 — Windows policy | **Blocked — owner-approved deferral for this sync.** Do not adopt managed update checks, organization-name display, or ADMX/ADML templates. Preserve current update behavior. See the [owner disposition record](https://linear.app/intgral/issue/INT-1454#comment-5b68d6cf-6587-49b1-bc37-a359986312c2). | Before resuming, define which user choices an administrator may override and which deployment templates IntCAD will support. |
| D5 — STEP move | Independent technical review passed at c5; see [acceptance §11.15](acceptance.md). The ledger records K1 as Adopted and the SEC-2 linear decoder as present, but the D5 row still contains the old K1 condition. | Reconcile that wording and complete final-SHA kernel/STEP checks, `cargo xtask layers`, `mesh-decisions`, `lists`, and related rule checks. This is an engineering gate, not a request to waive it. |

## Separate unresolved gates

- **Union naming:** retain the strict persisted-reference contract. Run `44675835996` did not pass; do not integrate the fallback. Reconsider only with a repair that passes the existing contract or a separately approved change to scope.
- **A2 / A6:** shared control crate adoption and desktop wiring are separate. The crate's refusal drain is not evidence that the app-local refusal has it. Record the residual separately; do not infer completed adoption.
- **A1:** reconcile the historical “crate split deferred” wording: the candidate contains `crates/mcp-server`. [Acceptance §5a](acceptance.md) records the MCP review claim and its evidence limits. The decision not to adopt the upstream `cancel` tool or `batch.path` field does not imply that the server split is missing.

The issue's tracker state is not owner acceptance. Final CI, persistence, platform evidence and final owner sign-off remain separate.
