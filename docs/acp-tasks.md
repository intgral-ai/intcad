# Workflow ACP tasks

A Mastra workflow can delegate one bounded task to an ACP agent the user already
configured (Codex, Claude Code, and the other discovered agents). The host side is
`HostAi::agent_run`; the implementation is `crates/ui-egui/src/agent_chat/host_ai/task.rs`.

## Ownership

- The task executor, not the chat panel, owns the process, the ACP session and the
  authentication exchange. Panel event adaptation stays in `mastra_backend.rs`.
- A task always opens a new ACP session. It may reuse only this executor's idle,
  healthy process, and only when every incoming update matches the active upstream
  session. A peer that omits session identities is never reused on that basis.
- The direct-chat session and its history are a different process and are never used
  for a task.
- The agent's own account owns its credentials. HostAi reads no API-key entry for a
  task and keeps no second credential store.

## Authentication

`connection.describe` starts a real ACP session and reports its actual authentication
methods. Runtime readiness alone never changes a connection to authenticated. If the
agent asks for authentication, the existing panel identity and sign-in path are used;
the host forwards the agent's own methods and never stores a key.

## Proposal mode

The task session receives no CAD MCP bridge and no host control token. Its working
directory is a fresh, empty temporary directory. Its system note asks for one JSON
proposal. The host accepts exactly:

```json
{"source":{"documentId":"<id>","revision":"<canonical u64>"},"commands":[{"command":"<name>","params":{}}]}
```

The host checks the source stamp, the 1..64 command bound, command names and
parameter objects, the allowed-command set and the remaining command budget before
returning the task result. Invalid output is `INVALID_PROPOSAL`. A task never applies
a command: ACP text, tools, plans and permissions are progress only, never evidence
that a CAD command ran.

This is a proposal-only capability boundary, not a general OS sandbox. The agent is an
ordinary child process running with the user's account and can reach whatever that
account can reach.

## Bounds and outcomes

One turn admits at most four ACP delegations. One task is clamped to at most 600
seconds. Those bounds cover host delegations only; they are not bounds on the external
agent's hidden model calls.

Authentication required, unsupported capability or model, cancellation, permission
denial, agent exit, timeout and invalid proposal each have a distinct outcome. Usage is
`null` unless the agent itself supplies it. Cancellation sends `session/cancel`, answers
open permission requests as cancelled, then stops the process tree after a short grace
period if the agent does not acknowledge.
