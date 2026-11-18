---
name: verify-intcad
description: Verify Intcad changes at the relevant product entry point before PR handoff, or maintain the repository verification map and desktop smoke test.
---

# Verify Intcad

Read the PR diff and the [validation policy](../../../docs/validation-policy.md). Use the
actual PR base for `git diff <base>...HEAD` and account for any worktree edits. Match the
changed behavior to the [feature map](features/README.md), including affected flows outside
the map. Define what a pass looks like before driving the app. For a fix, reproduce the old
failure where practical. Check the main path and the error, cancellation or persistence case
the change could break. Use the cheapest check that reaches the affected product boundary.

## Launch

Use a task worktree with its own Cargo target. The [desktop smoke recipe](features/model-and-files.md)
builds the app and runs a disposable instance. For other control-driven journeys, launch that build
with a free loopback port and a fresh absolute settings directory. Follow the
[control protocol](../../../docs/control-protocol.md) for authentication. Give recovery, crash,
control, candidate and validation output separate scratch paths. Use synthetic designs.
When normal startup or packaging is the behavior under test, use that startup or installed
build. Explicit `--control` suppresses normal startup flows such as the start page.

## Doctor

Check the running build with `ui.help` and use `ui.inspect` to confirm the viewport or surface
is ready. Authenticate against the new instance's own token and confirm its process is alive.
For package checks, record the artifact identity and installed version. Confirm any display,
WebView, fixture or provider needed by the journey before driving it.

## Drive

Use the public command or tool for command behavior and the actual UI control for UI behavior.
Use named handles from `ui.handles` or `ui.rects` for egui controls, and native input or
accessibility for Assistant's WebView controls. Wait for expected state with a bounded read-only
poll. If a mutation times out, inspect its outcome before retrying. Read the resulting model,
file or report after the action.

## Evidence

In PR Validation, give the checked commit and platform, exact command or entry point, expected
result, observed result and artifact link. Describe any worktree changes present during the run.
Capture a screenshot or recording for visual behavior and a measurement, file or report for side
effects. Keep artifacts outside Git and exclude private models and credentials. State failures,
blockers and unverified paths. Rerun when the relevant inputs change.

## Cleanup

Quit the instance you started, verify it exited and remove its scratch profile and control
files. Keep the evidence. The desktop smoke helper owns its app process and temporary profile.

## Maintain the map

Update the recipe in the same PR when an entry point, fixture or expected result changes.
During dedicated maintenance, check each recipe against source and replay one journey per
mapped feature. Report product regressions and blocked journeys. Put proven verification
corrections in one maintenance PR with evidence from replaying the corrected recipes.
