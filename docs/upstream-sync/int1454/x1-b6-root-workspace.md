# X1 / B6: root Cargo workspace evidence

This compares the root files, not the separate workspace under `solidcore/`.
The pins are pre-sync IntCAD `0fd92bea2`, frozen upstream `e151d5ea`, and
isolated candidate `db0b8dd35`. PR #185 head `c5f2f42fe` has the same three
root blobs as the isolated candidate. That identity does not make their CI
runs interchangeable.

| Pin | `Cargo.toml` blob | `Cargo.lock` blob / package records | `deny.toml` blob |
| --- | --- | --- | --- |
| `0fd92bea2` | `4bf3f4008ab2` | `b2e72c91d169` / 716 | `11481b2f752e` |
| `e151d5ea` | `af90f8bed4fe` | `b9628d9660bf` / 815 | `238928778109` |
| `db0b8dd35` and `c5f2f42fe` | `67e509e00cc5` | `0bffef42f749` / 839 | `02af8fdff85b` |
| Landed toolchain PR #180 (`3cb29651e`) | `4bf3f4008ab2` | `b2e72c91d169` / 716 | `11481b2f752e` |

| Root file / owner | Source disposition | Evidence or remaining decision |
| --- | --- | --- |
| `Cargo.toml` — B6 / K0 / D5 | Candidate keeps IntCAD workspace identity and the same member globs. It excludes `solidcore/` as a workspace member, points to the vendored kernel by path, and drops the retired `solidcraft-step` workspace entry. It keeps local DFM and Agent-task dependencies. | K0/CD-7 covers nested tree identity; D5 covers the scoped STEP migration. The B3 local all-target check exercised only the exported kernel, not this root workspace. Keep the root workspace and same-head integration gates separate. |
| `Cargo.toml` — B5 / B6 / A3 / A2-U8 / CD-6 | Candidate retains egui/eframe/egui-wgpu `0.36` and the `vendor/accesskit_macos` patch. Frozen upstream uses `0.37` and does not carry that patch. Candidate does not declare upstream plugin, `control-egui`, or extracted UI/brand crates at the root. | Do not mark the egui bump or vendor transition adopted: any move to `0.37` needs a separate disposition and desktop/wasm/macOS evidence. Plugin and brand exclusions stay in effect; the `control-egui` non-adoption is tracked under U8. Manifest absence does not clear adapted UI source provenance. |
| `Cargo.lock` — B6 / K0 / D5 / U17 | Pre-sync lock contains two Git-source package records; candidate and frozen upstream contain none. Candidate lock is not an upstream copy: 839 versus 815 package records. Candidate resolves egui/eframe/egui-wgpu `0.36.2`, `accesskit_macos` `0.26.3`, and `pgp` `0.21.1`, but no `wasmtime` package. | Locked builds attest their own source and target, not adoption of the upstream lock. Keep the final root locked build, Cargo inventory, and source-rights review separate. `pgp` is present for design provenance; source and advisory evidence alone do not close signature behavior or packaging acceptance. |
| `deny.toml` — B6 / CD-7 / design provenance | Candidate removes the SolidCore Git-source exception; unknown Git sources and registries remain denied. Candidate and frozen upstream have the same parsed deny policy, including the two OpenPGP-related advisory exceptions (`RUSTSEC-2023-0071`, `RUSTSEC-2026-0387`). These are absent in the pre-sync baseline. | [c5 advisories job](https://github.com/intgral-ai/intcad/actions/runs/44505747549/job/135497237938) passed advisories only; its combined advisories/bans/sources step was skipped. [db0 attempt 2 advisories job](https://github.com/intgral-ai/intcad/actions/runs/44811790548/job/136590887099) passed all three checks under the candidate policy. This proves neither that the exceptions are justified nor that licence checks ran. Retention requires review of the exact exceptions against the approved production verification/signing boundary; no broader waiver. Repeat the applicable gate on the final input. |

PR #180 landed the toolchain pin and CI wiring only; it did not change any
of these three blobs relative to the pre-sync baseline. The X1 nested tree
receipt and the B3 standalone check do not close any root Cargo policy
decision. No dependency, source allowance, advisory exception, or licence
policy is changed by this crosswalk.

The [target-specific OpenPGP resolution](b6-openpgp-resolved-graph.md)
records the distinct consumer and feature evidence for the two advisory
exceptions. Neither exception is approved by that evidence.

Reproduce blob IDs with `git rev-parse <pin>:<path>`, parse the TOML at each
pin with `git show <pin>:<path>` and `tomllib`, and count `[[package]]`
records by `source`. Policy equality compares parsed TOML, not comments.
Check the two linked jobs' actual step conclusions, not just job status.
