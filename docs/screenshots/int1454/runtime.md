# INT-1454 runtime acceptance (candidate d5d915400)

Desktop binary built from `worktrees/int1454` (`cargo build -p solidcraft`, exit 0,
20m41s). Window: Xvfb `:99` 1600x1000, Vulkan (radv), X11 backend.

| Step | Control call | Result |
|---|---|---|
| auth | `control.capabilities` | ok, guardedExecution 1.0 |
| open | `engine.execute doc.open examples/regressions/mirror_from_object_lid.json` | ok: `errors:0`, `features:6`, `name:"lid"` |
| rotate | `ui.drag (450,900)->(600,950)` | ok; frame changed |
| select | `ui.click (800,500)` + `selection.get` | count 1, face `F2:side:c1`, body `Body1`, cylinder r22, area 1512.88 mm² |
| UI view | `ui.selection` | selection `{body:Body1,index:3,type:face}` |
| Assistant | `ui.agent_chat {open:true}` | panel open, `features:true`; webview blank under Xvfb (headless WebKit limit) |

Screenshots: `01-start.png`, `02-open.png`, `03-rotated.png`, `04-face.png`,
`05-assistant.png`; replies in `summary.json`.

## Licence state of the candidate

- `crates/theme`: replaced with IntCAD's MIT/Apache implementation, same 44-root
  API; check 0, 6 tests, app check 0.
- `crates/look`: replaced with IntCAD's MIT/Apache implementation, same six-module
  API; check 0, 8 tests, `licences --check` passes.
- `crates/ui-kit`: still the restricted upstream crate. This is the only brand
  crate left; the desktop app cannot be distributed while it remains.
- `crates/doc`: conservative newer-file refusal tests; `doc --tests` check 0,
  6 newer-file tests pass.
