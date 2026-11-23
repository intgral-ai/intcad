# Controlled-operation stop budgets

No release stop bound is verified by preflight. Each listed capability starts **UNVERIFIED** for every controlled entry. An unmeasured operation or batch path must refuse with `UNSAFE_TO_START`, not inherit a bound from a superficially similar operation.

| Capability | Baseline status |
| --- | --- |
| Chat | UNVERIFIED |
| Harness | UNVERIFIED |
| Review | UNVERIFIED |
| DFM | UNVERIFIED |
| MCP | UNVERIFIED |
| Control | UNVERIFIED |
| AI autocomplete | UNVERIFIED; reserved only |
| Design Intent | UNVERIFIED; reserved only |

For a verified entry, record the revision, platform, operation class, safety boundary, worst-case stop time and supporting test evidence. Include waits for user permission, queued work, in-flight local mutation and late results where applicable. A normal renewal interval is not evidence of a stop bound. Ordinary free CAD is outside this table.
