# Controlled-operation stop budgets

Preflight does not verify a stop bound. Every controlled capability/entry pair starts **UNVERIFIED** unless explicitly listed as verified below. A pair without a verified bound must refuse with `UNSAFE_TO_START`, not inherit a bound from another pair. These are envelope claims, not guarantees that every workload on a listed entry fits.

| Capability | Production status |
| --- | --- |
| Chat | UNVERIFIED |
| Harness | **VERIFIED: 280 s**, `Mastra` only; see [the controlled-entry record](control-safe-stop-bound.md). |
| Review | **VERIFIED: 600 s**, desktop `ReviewPanel` only; every other Review entry is UNVERIFIED. See [the Review evidence record below](#desktop-review-panel). |
| DFM | **VERIFIED: 280 s**, `CliDfm` only; see [the controlled-entry record](control-safe-stop-bound.md). |
| MCP | **VERIFIED: 280 s**, `HeadlessMcp` and `GuiBridgeMcp` only; `ExternalAcpMcp` is UNVERIFIED. See [the controlled-entry record](control-safe-stop-bound.md). |
| Control | **VERIFIED: 280 s**, `DirectControl` only; `ExternalUiAutomation` is UNVERIFIED. See [the controlled-entry record](control-safe-stop-bound.md). |
| AI autocomplete | UNVERIFIED; reserved only |
| Design Intent | UNVERIFIED; reserved only |

For a verified entry, record the revision, platform, operation class, safety boundary, worst-case stop time and supporting test evidence. Include waits for user permission, queued work, in-flight local mutation and late results where applicable. A normal renewal interval is not evidence of a stop bound. Ordinary free CAD is unaffected and does not consult this table.

## Desktop Review panel

- **Revision and evidence:** Measured at revision `11a3f345`; the [DFM Review safe-stop measurement](../dfm-review-safe-stop-bound.md) was recorded in commit `4d9bcdb74e`.
- **Platform and operation class:** Linux x86_64, AMD Ryzen 7 7800X3D (16 logical CPUs). One desktop Review panel `dfm.check` run, from project freeze and snapshot through queue admission, native rebuild, sheet and flat-overlap checks, and findings writes. The measurement excludes paid admission, renewal, ledger and settlement calls.
- **Safety boundary:** The measured models cover up to 32 B-rep cut features, 4 sheet bodies and 2,080 cosmetic holes. Native rebuild cost grows roughly quadratically with B-rep cut-feature history. Larger models are outside the measured envelope; this budget entry does not enforce an input size limit.
- **Worst observed stop time:** 96.510 s for the 32-cut model under host load. For the sample bracket, small plate, large perforated sheets and 32-cut rebuild, respectively, the five-pass medians were 0.079 s, 2.228 s, 0.209 s and 33.174 s; the worst of those passes were 0.082 s, 2.268 s, 0.246 s and 35.911 s.
- **Verified ceiling and lease:** The 600 s bound is the engine's cooperative execution ceiling, not a hard preemption guarantee. The run window is the earlier of that ceiling and the remaining authorization, so a contract rev4 300 s lease still cuts the window to at most 300 s.
