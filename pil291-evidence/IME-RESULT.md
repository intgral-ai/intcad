# Manual IME and paste verification on the official Nix wrapper (2026-11-14)

Build under test: candidate 2, official wrapper
`/nix/store/lylbk0nfi28yb2xg85kpbihcd8s2dnzg-solidcraft-0.1.0-beta.3/bin/solidcraft`
(`.solidcraft-wrapped`), PID 755505, control 46644.

Wrapper environment as launched:

- `GTK_IM_MODULE=fcitx`
- `GTK_IM_MODULE_FILE=/nix/store/lylbk0nfi28yb2xg85kpbihcd8s2dnzg-solidcraft-0.1.0-beta.3/lib/gtk-3.0/3.0.0/immodules.cache`
- `GTK_PATH` begins with `/nix/store/k9ym8g6l5jzxy2masn07ghx07l4f1plc-fcitx5-gtk-5.1.7/lib/gtk-3.0`
- Log `/tmp/opencode/wrapper-final.log` has no `GLib version too old` and no
  `Loading IM context type 'fcitx' failed`.

## What the user did and what the app shows

The Assistant page, captured from the live web view after the manual test, shows the
sent bubble:

    你好hello-paste) 已经完全恢复正常

and one tool card reading `(cancelled)`. The composer is empty afterwards, the model
selector shows `deepseek-flash`, and the page header shows `Ready`.

Interpretation:

| Step | Input | Observed result |
|---|---|---|
| Shuangpin preedit/commit | `nihao` → `你好` | Chinese committed into the composer (candidate selection worked) |
| Paste | `hello-paste` (ASCII) | Pasted into the composer; no freeze, no black window |
| More Chinese | `已经完全恢复正常` | Typed by Shuangpin in the same composer |
| Send | the combined message | Bubble added to the transcript; the run's tool card shows `(cancelled)` (user cancelled the run) |

The app stayed responsive throughout: the control channel answered, the process stayed
in `do_epoll_wait`, and no hang (the earlier Ctrl+V freeze) occurred.

## Conclusion recorded here

- On the official wrapper the fcitx5 GTK module loads and Shuangpin input, candidate
  selection, commit, ASCII paste and a following Chinese send all work on the real
  desktop.
- The earlier "Ctrl+V hangs the app" observation did not reproduce in this manual run;
  the paste path is no longer a blocker for this build.
- The IME investigation is closed; the wrapper launch method above is kept.
- Remaining desktop blocker for PIL-303/304 is the separate web-view placement defect
  (page drawn at the window's top-left instead of the Assistant panel rect), tracked on
  its own.
