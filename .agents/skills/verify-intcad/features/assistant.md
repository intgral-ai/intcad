# Desktop Assistant

## Sub-features

Open and close the native panel, type or click in its WebView, connect an agent and observe a
message or tool result.

## How to get to it (user POV)

Use the Assistant button or its shortcut. Use the panel's Close control to close it and
reopen it from the app. For an agent or tool change, select the intended agent and send a
request against a disposable design.

## Check the bundled message box in Chrome

For a composer change, run the [Bombadil spec](../browser/composer.ts)
from the repository root with Chrome and [Bombadil 0.7.10](https://github.com/antithesishq/bombadil/releases/tag/v0.7.10) on PATH.
Use a fresh output directory outside the checkout.

```sh
python3 .agents/skills/verify-intcad/browser/run.py --output /tmp/intcad-composer-run
bombadil browser inspect /tmp/intcad-composer-run
```

Bombadil opens the bundled page on loopback. A synthetic desktop host supplies an idle
Assistant with no attachments and records outgoing messages. Bombadil types spaces, presses
Enter, adds generated text and submits by click or Enter. Check that Send follows the draft,
whitespace produces no message, and each valid submission sends the exact draft once and
clears it. The run must complete both submit routes. The trace records each action, observed
state and screenshot. To replay a failure on the same build and port, run:

```sh
python3 .agents/skills/verify-intcad/browser/run.py --reproduce /tmp/intcad-composer-run/trace.jsonl --output /tmp/intcad-composer-replay
```

The Assistant UI workflow runs this check for PRs touching the page or its verification files.
Use the native journey below for embedded WebView focus, window layout, native IPC handling
and provider behavior.

## Driving it with the native desktop

Follow [Agent chat](../../../../docs/agent-chat.md) for the entry points. Confirm the app's
`ui.agent_chat` state through the [control protocol](../../../../docs/control-protocol.md).
For pointer or keyboard changes, use native input or accessibility on the WebView. On Windows,
the [UIA helper](../../../../scripts/e2e/windows_assistant_uia.py) can locate the page and its
Close button. Verify that the panel disappears and returns on reopening. For agent or tool
changes, verify which agent ran, the visible turn or tool result and the resulting design state.

## Gotchas

The app screenshot can omit the child WebView. Use native window capture for page evidence.
For packaging or sign-in changes, use the affected installed build and an authorized test
account. Record the provider and runtime that actually ran. Keep credentials out of evidence.
