# Workflow ACP tasks (HostAi slice)

This document records the **HostAi-level** task executor behind the protocol's
`agent.run`. #4 is paused by DQ-11 B-minimal; the task path is not a Beta capability.
The Direct ACP chat path is unaffected.

- The panel still refuses `(workflow, ACP)` with an actionable message. Direct chat with
  that agent keeps working. The runtime call, sign-in event and permission UI remain a
  future ACP decision, not part of this delivery.
- The task executor, not the chat panel, owns the ACP process, session, authentication,
  cancellation and permission state. Panel event adaptation stays in
  `mastra_backend.rs` for a future wiring change.

## Ownership

- A task always opens a new ACP session. It may reuse only this executor's idle,
  healthy process, and only when every incoming update matches the active upstream
  session. A peer that omits session identities is never reused on that basis.
- The direct-chat session and its history are a different process and are never used
  for a task.
- The agent's own account owns its credentials. HostAi reads no API-key entry for a
  task and keeps no second credential store.
- A stale permission key is refused: permission keys are process-wide unique and only a
  key this task issued can be answered.

## Authentication

`HostAi::describe_agent` starts a real ACP session and reports the agent's actual
authentication methods. Runtime readiness alone never changes a connection to
authenticated. The panel does not yet render those methods or call the agent's
`authenticate` request from the workflow path; that is the follow-up UI wiring.

## Proposal mode

The task session receives no CAD MCP bridge and no host control token. It gets one
empty host-owned working directory per task process; a reused process keeps that same
empty directory. Its system note asks for one JSON proposal. The host accepts exactly:

```json
{"source":{"documentId":"<id>","revision":"<canonical u64>"},"commands":[{"command":"<name>","params":{}}]}
```

The host validates the JSON shape, source stamp, 1..64 command count, command-name
allowlist when one is supplied, per-command parameter size and the numeric command
budget before returning the task result. Invalid output is `INVALID_PROPOSAL`.

Not yet validated here: discovered command schemas, requirements artifacts and the
real remaining-turn budget. Those belong to #9 and #11. A task never applies a command:
ACP text, tools, plans and permissions are progress only, never evidence that a CAD
command ran.

This is a proposal-only capability boundary, not a general OS sandbox. The agent is an
ordinary child process running with the user's account and can reach whatever that
account can reach.

## Bounds and outcomes

One turn admits at most four validated ACP delegations. One task is clamped to at most
600 seconds; session start and model selection have their own 30-second ceiling.
Cancellation is polled while waiting for session start, model selection and the prompt
reply, so a Stop during `session/new` sends no prompt. These bounds cover host
delegations only; they are not bounds on the external agent's hidden model calls.

Authentication required, unsupported capability or model, cancellation, permission
denial, agent exit, timeout and invalid proposal each have a distinct outcome. Usage is
forwarded only when the agent returns a numeric usage object, otherwise it is `null`.
Cancellation sends `session/cancel`, answers open permission requests as cancelled, then
stops the process tree after a short grace period if the agent does not acknowledge.
