# Desktop Assistant

## Sub-features

Open and close the native panel, type or click in its WebView, connect an agent and observe a
message or tool result.

## How to get to it (user POV)

Use the Assistant button or its shortcut. Use the panel's Close control to close it and
reopen it from the app. For an agent or tool change, select the intended agent and send a
request against a disposable design.

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
