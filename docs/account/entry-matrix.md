# Client authorization entry matrix

Classification comes from the host dispatch path, never from a caller-supplied method parameter or request origin label.

| Entry | Baseline seam | Authorization and billing |
| --- | --- | --- |
| Human desktop CAD, ordinary completion, browser CAD | `crates/ui-egui/src/control.rs`, `apps/solidcraft-web/src/api.rs` | Free, offline. An ordinary human action stays free when it shares a dispatch helper. |
| Ordinary CLI file handling and non-DFM validation | `apps/solidcraft-cli/src/main.rs`, `apps/solidcraft-cli/src/validation.rs` | Free, offline. |
| Built-in chat and harness, including custom model and ACP setup | `crates/ui-egui/src/agent_chat` | Online eligibility. Pure text does not consume credits; each external Intcad logic operation does. |
| Trusted runtime bridge | `crates/ui-egui/src/agent_chat/host_bridge.rs`, `task_runtime.rs` | Preserve the host-created operation context when forwarding internally. Do not debit twice. |
| Headless MCP, GUI bridge MCP, external ACP MCP | `apps/solidcraft-cli/src/main.rs`, `crates/mcp/src/server.rs`, `crates/mcp/src/tools.rs` | Online eligibility plus one unit per successful logical operation, including read-only queries; outer batch is one operation. |
| External control and external UI automation | `apps/solidcraft/src/control_server.rs`, `crates/ui-egui/src/control.rs`, `execution_guard.rs` | Keep the connection token and local permission checks. Online eligibility; bill actual logical operations, not handshake, navigation or status. |
| DFM from Review or CLI | `crates/ui-egui/src/dfm_review.rs`, `apps/solidcraft-cli/src/validation.rs` | Online eligibility and one unit per completed logical check; refuse a mixed CLI request before running if its DFM check is not admitted. Viewing or attaching an existing report is not a rerun. |
| Unknown source or command class | Native trust boundary | Deny by default; no interactive login window on a programmatic request. |

The matrix describes intended gates, not evidence that these entry points are already guarded.

Open-source builds released before the account authorization system existed never had controlled capabilities under that system. They cannot be revoked or disabled remotely. Version admission and revocation apply only to controlled capabilities in official builds that installed an account service. This policy does not provide or promise remote control over historical open-source builds.
