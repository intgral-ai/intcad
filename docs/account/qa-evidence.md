# Account QA evidence

This page preserves the desktop account-entry branch's local test snapshot and the entitlement
branch's separate Linux guest run. Neither is evidence for the combined revision. Neither used
live credentials or a staging or production account service.

## Tested revision and environment

| Item | Value |
| --- | --- |
| Client commit | Final fix on top of `e1b3ebc0e`; base Task 1 code `4fb4859` |
| Base | `ad9ab83f8` |
| Branch | `feat/int1497-desktop-entry` |
| Worktree state | clean after the final fix commit; logs under `.superpowers/` and `/tmp/opencode` |
| Platform | Linux host (native `x86_64` build of the client crates) |
| Package type | none; no installer or portable package was built or installed |
| Target | this worktree's own native `target/`; no shared or reused build directory |

The server side was read only at `23ab4119ba0652f2d35cc0bda0fc35d8154063d6`, clean and unbuilt.
Contract byte evidence is in [`contract-pin.md`](contract-pin.md).

## Results

| Check | Result | Evidence |
| --- | --- | --- |
| Frozen rev3 embedded bytes match client `contracts/pin.json` | **PASS** (earlier Task 13 evidence) | `contract-pin.md` |
| Frozen `contract_vectors` (offline) | **NOT_RUN** in Task 1 | requires the external frozen artifacts; unchanged by this task |
| `solidcraft-account-native` library tests (offline) | **PASS**, 108 passed / 0 failed / 1 ignored | `cargo test --offline --locked -p solidcraft-account-native --lib` |
| Desktop account tests (`apps/solidcraft`) | **PASS**, 35 passed / 0 failed | `cargo test --offline --locked -p solidcraft --bin intcad account::tests` |
| Account UI tests (`solidcraft-ui-egui`) | **PASS**, 30 passed / 0 failed | `cargo test --offline --locked -p solidcraft-ui-egui --lib account::` |
| Chat/Review inline code, browser, cancel and retry controls | **PASS** | focused UI account tests; the Chat strip reachability test still passes |
| Browser launch requires platform-provenance activation | **PASS** | a platform pointer click dispatches; an app-injected control click is refused |
| Startup recovery for a readable sign-out tombstone | **PASS** | readable marker offers recovery, no identity/authority restored; unreadable storage stays closed |
| Bounded repaint while a sign-in or sign-out worker is pending | **PASS** | pure scheduling check; no repaint is requested once idle |
| Production web shell sign-in prompt and Review prompt reachability | **PASS**, 1 each | focused UI tests for the native web-view strip and an `UNSAFE_TO_START` Review refusal |
| English/complete-catalog coverage for new messages | **PASS** | `... i18n::tests::every_complete_catalog_carries_every_english_message --exact` |
| Focused clippy (`-D warnings`) for native, UI and desktop crates | **PASS** | `cargo clippy ... --all-targets -- -D warnings` |
| `cargo xtask lists` | **PASS** | pending locale file folds and catalogs stay sorted |
| `cargo xtask layers` | **PASS**, no layering violations | native-only closure guard passed |
| `solidcraft-ui-egui` wasm library check | **PASS**, 0 errors (existing warnings) | `cargo check ... --target wasm32-unknown-unknown --lib` |
| Full `solidcraft-ui-egui` library suite | **FAILED**, same pre-existing `TexturesDelta`/agent-discovery signatures | representative failures reproduced at base `ad9ab83f8` (see Limits) |
| Full desktop app test binary | **FAILED**, 184 passed / 2 failed | both are the pre-existing `TexturesDelta` drop-panic signature |
| Linux direct-binary package build and install | **NOT_RUN** | no package built in this bounded checkpoint |
| Live WorkOS device-code completion and product verification | **NOT_RUN** | no production/provider credential, service or remote write |
| No-auto-browser / explicit-click behavior on a real desktop | **NOT_RUN** | covered by scripted seams only; no GUI session |
| Linux real keyring / credential persistence | **NOT_RUN** | scripted in-memory stores only; no OS keychain exercise here |
| Windows MSI install, Credential Manager and account flows | **NOT_RUN** | no Windows package or authorized Windows environment used |
| macOS package install and account flows | **BLOCKED** | no authorized macOS acceptance environment or package |
| GUI account menu, Chat/Review prompt layout and compact width | **NOT_RUN** | no clean-base GUI/visual session |
| Live account billing, settlement or production authority | **BLOCKED** | deliberately not implemented or configured |

## Limits of this evidence

- The new tests use scripted device/product transports and in-memory credential stores. They
  prove the attempt fence, explicit browser action, platform-provenance refusal of automation,
  verification-before-persistence, startup tombstone recovery and UI states, not a live provider,
  server, staging, keychain or production authority.
- The platform-provenance tests feed a scripted platform pointer event and a scripted
  control-channel click into the same frame loop. They prove the app's defensive dispatch; they
  do not claim an OS-level input boundary or an acceptance run on a real desktop.
- The full UI suite's `TexturesDelta` failures are a harness panic already present at base
  `ad9ab83f8`: `dialogs_hole::tests::every_dropdown_option_previews_a_different_hole` and
  `agent_chat::discover::tests::desktop_path_can_start_an_agent_and_its_runtime` were reproduced
  individually at the base revision with the same signatures. The other failures share those two
  signatures; they are not caused by this change but were not all rebased individually.
- The library run is Linux-host evidence only. It is not a Windows, macOS, packaging or GUI
  result. The wasm check proves the UI library still compiles; it is not a browser run.
- No package, installer or artifact hash is reported because no package was built. No entry or
  capability stop bound is reported because none was measured.
- `docs/account/capability-budget.md` therefore keeps every capability **UNVERIFIED**;
  `docs/account/entry-matrix.md` keeps its intended-gate wording.
- Existing production authority remains disabled: no trusted build-time WorkOS client id, AuthKit
  origin or product origin is set in this build, no installation bootstrap exists, and there is no
  real keychain validation, refresh/revocation handling or settlement authority. The
  [build configuration](desktop-configuration.md) seam and its fail-closed validation are tested
  with synthetic values only.
- The durable activation commit is a single key. Deterministic fake-store tests cover a blocked
  commit, a failed commit marker, a superseded generation, a tombstone racing a blocked commit,
  repeated sign-out while pending, a failed sign-out marker and display-only identity. Partial
  writes are non-restorable; a cancel/sign-out during promotion is refused rather than waited for.
  An accepted sign-out immediately revokes in-process authority, but only confirmed background
  cleanup completes sign-out. Abrupt termination before the worker writes the durable tombstone
  may leave the prior committed identity restorable. Normal exit joins background account
  workers and may wait on the OS keyring. Real OS keyring latency and destructive crash
  acceptance are **UNVERIFIED**.

## Linux desktop account-entry run

This run tested clean client commit `c8c5c392866a12274387ce66833109331e530309` in the
Linux guest for task `int1497-entry`. Its direct binary and deployed executable matched SHA-256
`f07fe3861559a38f9aa8ebe362000606bb55c445c98c5cd1bd9eb7e5be3e4de5`.
The local evidence directory is
`~/.local/state/intcad-e2e/runs/intcad-e2e-int1497-client-778ee328d234-int1497-entry/20261121T235143Z-65612/`.

| Check | Result | Evidence |
| --- | --- | --- |
| Build and deploy the Linux direct binary | **PASS** | `report.json`, steps 1–2 |
| General guest acceptance, steps 3–8 | **BLOCKED** | `ui.documents` refused the request because the controlled entry had no verified stop bound; `logs/20-ui-documents.json` |
| Signed-out desktop account entry in the guest | **BLOCKED** | No persistent default credential store or complete account-service configuration; `screenshots/20-account-normal-before.png`, `screenshots/21-home-account-unavailable.png`, and `logs/21-secure-store-dbus.txt` |
| 700×600 compact account entry in the guest | **NOT_RUN** | Desktop clamped the request to 960×600; `screenshots/22-resize-clamped.png` and `screenshots/23-file-menu-960x600-home-closed.png` are not evidence at 700×600 |
| Anonymous sample display and view change | **Observed** | Manual view-axis click changed the rendered view; `screenshots/25-sample-view-control.png`. This does not pass the automated CAD suite. |
| Real sign-in, registration, verification, or password recovery | **NOT_RUN** | No account configuration was supplied and no external account request was made. |

The 700×600 regression remains a UI harness test, not a native-window screenshot. The overall
guest acceptance result is **BLOCKED**. The guest was stopped after capturing the evidence.

## Remaining gates

1. Reviewed WorkOS client id, AuthKit verification origin and product HTTPS origin set by the
   trusted release builder, then a real device-code completion with the exact configured origins.
2. Installation bootstrap and startup tombstone recovery on a real desktop install (including
   readable-marker recovery and the unreadable-storage refusal).
3. Linux direct-binary packaging and the full account flow matrix on a real install.
4. Windows MSI packaging, Credential Manager behavior and durability/ACL checks.
5. macOS package and account flow in an authorized environment, or keep the release gate RED.
6. Real OS keychain integration and GUI account-menu/Chat/Review prompt verification against a
   clean baseline revision, including no auto browser launch.
7. Measured stop budgets for every controlled entry, including waits, queued work and late
   results.
8. Staging/live account authority, settlement, restart recovery and revocation revalidation;
   none is claimed here.
