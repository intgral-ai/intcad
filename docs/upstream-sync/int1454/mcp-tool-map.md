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

## Original disposition proposal

1. `version` (ReadOnly): adopt. Cheap, read-only, and the MCP unit needs a way to report the
   build; it does not change another tool.
2. `cancel` (Edits): adopt only if the agent-task controller in IntCAD has a cancellation path to
   connect it to; otherwise defer with a recorded reason. It changes what an agent can stop, so it
   is audited as its own item.
3. Existing tools: produce the per-tool schema diff from `tools.rs` (and the shared
   `mcp-server` machinery, if the split is taken) and classify each hunk as
   equivalent / behaviour change. Do this before the MCP PR; the map is a precondition, not a
   by-product.

## Candidate A1 disposition

Frozen upstream: `e151d5ea6866fb055e011379153fb1cae0566f22`. Candidate: `db0b8dd350bbdffed8927845c86c5028d280cb0e`. Implementation commit `6e1bcf53af8d001b838abb1545aa63aa55363bc7` is an ancestor of the candidate and records the three choices below. This is a source inventory of those choices, not a new Compatibility Decision.

| Upstream → candidate | Input and output | Effects, callers and disposition |
| --- | --- | --- |
| `version` → `version` | Closed empty-object input; success payload `{version, commit, mode}`. The mode is `app` or `headless`; unavailable commit is `unknown`. | **Adopted**, `ReadOnly`. A standalone MCP tool, not a batch step; reads metadata from the backend answering the session through `ui.help`. See `crates/mcp/src/tools.rs:89–96,1115–1124,1479` at the candidate. |
| `cancel` → absent | Upstream closed empty-object input; forwards the `control.cancel` outcome. No candidate registration or dispatch arm. | **Not adopted**. Upstream `Edits` tool for that MCP server's outstanding app request, not document undo. The implementation records that no reachable app cancellation route was identified. See frozen `crates/mcp/src/tools.rs:386–398,1000` and `docs/mcp.md:488–501`. |
| `batch` with optional `path` → `batch` with required inline `commands` | The upstream string `path` selects a server-read script as an alternative to inline steps; the candidate schema rejects this field at the MCP RPC boundary. The candidate still returns inline batch results and failure/rollback details. | **`path` not adopted**; the `Files` tool itself remains. The extra server-side file-read input was left for a separate CD; this record does not approve it. See frozen `crates/mcp/src/tools.rs:144–179,1509–1515` and candidate `tools.rs:165–197,575–580,1740–1743`, `crates/mcp-server/src/schema.rs:38–43,66–82`. |

The candidate MCP wrapper owns `solidcraft_mcp_server::Server<Agent>` (`crates/mcp/src/server.rs:197–206`). Both the CLI (`apps/solidcraft-cli/src/main.rs:934`) and the desktop chat bridge (`crates/ui-egui/src/agent_chat/host_bridge.rs:200–201`) use that wrapper. The shared RPC host checks arguments before dispatch (`crates/mcp-server/src/server.rs:255–263`). The old “crate split deferred” record does not describe this candidate.

No runtime contract pass is claimed here. The historical MCP-connect receipt in [acceptance](acceptance.md) is on a different workflow/source pair; §5a does not supply a source-pinned verdict for this entire MCP surface. The remaining tool-by-tool schema, output, side-effect and caller classifications and final-SHA verification remain open.

The [21-tool candidate contract audit](mcp-cd2-contract-audit.md) inventories the existing tools and identifies observable changes. It does not approve those changes or replace runtime verification.

## Callers to update with any tool change

`crates/mcp/src/server.rs` (dispatch), `crates/ui-egui/src/agent_chat` (bridge and gate method
list), `docs/mcp.md`, the agent skills under `skills/`, and the MCP tests (`crates/mcp/src/tests.rs`).
Case and `id`/`command` aliases already share one resolution path after PR #179.
