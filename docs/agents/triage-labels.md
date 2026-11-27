# Triage labels

The skills speak in five canonical triage roles. This table is the configured vocabulary for those roles in `intgral-ai/intcad`:

| Role | Label in our tracker | Meaning |
|---|---|---|
| `needs-triage` | `needs-triage` | Maintainer needs to evaluate this issue |
| `needs-info` | `needs-info` | Waiting on the reporter for more information |
| `ready-for-agent` | `ready-for-agent` | Fully specified, ready for an AFK agent |
| `ready-for-human` | `ready-for-human` | Requires human implementation |
| `wontfix` | `wontfix` | Will not be actioned |

These are configured label strings, not labels a run creates; a repository only has the labels a maintainer has added. Before applying one, confirm it exists (`gh label list --repo intgral-ai/intcad --search <label>`). If it is missing, ask a maintainer and do not create it yourself.

When a skill mentions a role (e.g. "apply the AFK-ready triage label"), use the matching label string from this table.
