# Desktop paid-entry safe-stop budgets

The desktop budget is derived from the [controlled entry measurement record](../controlled-safe-stop-bounds.md).
No workload was remeasured. The harnesses started from `7d926db8b247ceb042af5afbba7aa2b6366fbd1d`;
the entry shapes were audited at `ac22ef39f1`. The runs used one shared Linux x86-64
development host (AMD Ryzen 7 7800X3D), not a release or CI runner.

| Capability / entry | Budget | Recorded evidence and envelope |
| --- | ---: | --- |
| Control / `DirectControl` | 280 s | [Direct Control](../controlled-safe-stop-bounds.md#control--directcontrol): 96.514944 s composed proxy. One synchronous command, at most one comparable rebuild, small request/reply and ready dispatch. |
| MCP / `HeadlessMcp` | 280 s | [Headless MCP](../controlled-safe-stop-bounds.md#mcp--headlessmcp): 96.511649 s composed proxy. One `execute` without `expect`, at most one comparable rebuild, small JSON and responsive local I/O. |
| MCP / `GuiBridgeMcp` | 280 s | [GUI bridge](../controlled-safe-stop-bounds.md#mcp--guibridgemcp): 96.515708 s composed proxy. One forwarded command, at most one comparable rebuild, live turn and immediately serviced queue. |
| DFM / `CliDfm` | 280 s | [CLI DFM](../controlled-safe-stop-bounds.md#dfm--clidfm): 96.511214 s composed proxy, 37.255 s observed CLI-path maximum. One prepackaged candidate, one comparable rebuild and a minimal local profile, without external dependencies. |
| Harness / `Mastra` | 280 s | [Mastra](../controlled-safe-stop-bounds.md#harness--mastra): 96.612593 s composed proxy. At most one already-started comparable rebuild and a measured-size offline drain tail, with no new CAD step after cancellation. Not a measurement of a packaged Mastra turn. |

The common rebuild proxy is the load-stressed 96.5098 s run on a simple sheet
with 32 independent B-rep cut features. The measurements support an envelope,
not a mathematical bound, a runtime restriction on the model, or evidence
that every request on a listed entry fits. Review keeps its separate
[600 s evidence record](capability-budget.md#desktop-review-panel).

## One lease, with no runtime tolerance

The measurement record proposes 300 s, or 3.11× the 96.5 s load-stressed
proxy. The installed 280 s budget is 2.90× the same proxy. It is the
largest round value that fits one `MAX_ABSOLUTE_GRANT_LEASE = 300 s` lease
with 20 s set aside for admission transit. The client's `remaining_seconds`
already reflects round-trip time. The margin is built into the declared
bound, not granted at runtime: `remaining >= bound` remains strict.

The delivered time envelope is about 7% narrower than the recorded proposal.
Work declared to need 290 s is outside this envelope and must be refused
rather than admitted on a 280 s window. The gate checks the pair's declared
budget, not the runtime of an individual command. It cannot detect an
operation that actually needs 290 s if it presents as a 280 s operation.

The [shared Control gate](../../crates/ui-egui/src/control.rs) refuses an unverified pair, a missing or expired
authorization window, or remaining authorization below the declared bound.
Identity, account status, version, operation-state and billing checks are
unchanged. The result cutoff is the live grant expiry (or the earlier
forwarded cutoff, if present). `engine.batch`, including an MCP batch
forwarded as `engine.batch`, still reserves `N × step_bound` and rechecks
room for the remaining steps before each substep. This reserve is not evidence
for additional batch overhead or rollback.

**The bound is a declaration used for admission and settlement, not an enforced stop.**
A result arriving at or after the authorization deadline never commits.
Settlement still runs once under the original operation ID: a late effect is
deferred once for recovery, not charged or released on the expired path.
If work exceeds the declared bound and is still in flight when authorization
expires, it keeps running. Its side effects are not undone; only its result
is discarded.
This is a known execution-layer limitation, not a fix for the contract's
safe-stop requirement. `(Review, ReviewPanel)` is the only path that also
passes the clamped [`stop_at_ms`](../../crates/ui-egui/src/dfm_review.rs) into the run. Review checks that deadline
between checks, so it goes beyond admission and write-back checks but still
does not physically interrupt an in-flight rebuild.

Unbounded B-rep history, multi-step batches beyond the reserve, paid
settlement latency, slow I/O, queue and permission waits, UI-thread stalls
and packaged runtime teardown are outside the evidence. The table does
not change the separate headless MCP or CLI authority wiring or claim that
those production paths run through the desktop Control gate. The two
external entries have no measurement, and `Unknown` is refused by design.

Regression tests in `control.rs` exercise fresh and short grants, unverified
entries, the strict batch reserve and late replies with a deterministic
monotonic clock. The Review test checks admission with a lease shorter than
its bound. Desktop account tests assert the exact verified pairs.
