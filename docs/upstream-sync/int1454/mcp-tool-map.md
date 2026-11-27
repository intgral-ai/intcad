# MCP tool map — develop vs v0.44.1 (CD-2)

First pass for the CD-2 requirement ("map old name → new name, schema, behaviour, side effects,
permission, callers before touching tools"). Method: extract every `tool("…")` registration and
its `Effect::` class from `crates/mcp/src/tools.rs` on both refs
(`origin/develop` `0fd92bea2`, `v0.44.1` `e151d5ea6`).

## Names and permission class

| Tool | develop | v0.44.1 | Status |
| --- | --- | --- | --- |
| list_commands | ReadOnly | ReadOnly | unchanged |
| describe_command | ReadOnly | ReadOnly | unchanged |
| execute | Files | Files | unchanged |
| batch | Files | Files | unchanged |
| inspect_design | ReadOnly | ReadOnly | unchanged |
| measure | ReadOnly | ReadOnly | unchanged |
| body_topology | ReadOnly | ReadOnly | unchanged |
| set_parameter | Edits | Edits | unchanged |
| check_parameters | Edits | Edits | unchanged |
| screenshot | Files | Files | unchanged |
| compare_picture | Files | Files | unchanged |
| export | Files | Files | unchanged |
| crash_reports | ReadOnly | ReadOnly | unchanged |
| running | ReadOnly | ReadOnly | unchanged |
| pending | ReadOnly | ReadOnly | unchanged |
| undo | Edits | Edits | unchanged |
| redo | Edits | Edits | unchanged |
| new_design | Replaces | Replaces | unchanged |
| open | Files | Files | unchanged |
| save | Files | Files | unchanged |
| skill_reference | ReadOnly | ReadOnly | unchanged |
| **version** | — | ReadOnly | **new** |
| **cancel** | — | Edits | **new** |

Result: **no renames, no removals, no permission-class changes** among the 21 existing tools.
CD-2's "pure rename" path therefore does not apply; only two additions need a decision.

## Where the files differ

`diff -u` of the two `tools.rs` files: **54 hunks, 526 removed / 525 added lines**. So the
descriptions and argument schemas of existing tools were edited upstream even though their names
and effect classes did not change. Adopting them is a schema change that must be classified per
tool before merge (CD-2: semantically equivalent → migrate together; behaviour, permission or side
effect change → separate Compatibility Decision).

## Disposition proposed

1. `version` (ReadOnly): adopt. Cheap, read-only, and the MCP unit needs a way to report the
   build; it does not change another tool.
2. `cancel` (Edits): adopt only if the agent-task controller in IntCAD has a cancellation path to
   connect it to; otherwise defer with a recorded reason. It changes what an agent can stop, so it
   is audited as its own item.
3. Existing tools: produce the per-tool schema diff from `tools.rs` (and the shared
   `mcp-server` machinery, if the split is taken) and classify each hunk as
   equivalent / behaviour change. Do this before the MCP PR; the map is a precondition, not a
   by-product.

## Callers to update with any tool change

`crates/mcp/src/server.rs` (dispatch), `crates/ui-egui/src/agent_chat` (bridge and gate method
list), `docs/mcp.md`, the agent skills under `skills/`, and the MCP tests (`crates/mcp/src/tests.rs`).
Case and `id`/`command` aliases already share one resolution path after PR #179.
