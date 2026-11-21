# Account QA evidence

Bounded Task 1 evidence snapshot for the desktop account entry. It records only checks that
actually ran. No platform package, live credential, GUI session, real keychain, hardware, staging
or production service was used.

## Tested revision and environment

| Item | Value |
| --- | --- |
| Client commit | `4fb4859` (`feat(account): wire explicit desktop sign-in to verified product session`), plus the cancel-race fix and evidence commit on top of it |
| Base | `ad9ab83f8` |
| Branch | `feat/int1497-desktop-entry` |
| Worktree state | clean; these evidence changes are committed on the task branch |
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
| `solidcraft-account-native` library tests (offline) | **PASS**, 101 passed / 0 failed / 1 ignored | `cargo test --offline --locked -p solidcraft-account-native --lib` |
| Desktop account tests (`apps/solidcraft`) | **PASS**, 20 passed / 0 failed | `cargo test --offline --locked -p solidcraft --bin intcad account::tests` |
| Account UI tests (`solidcraft-ui-egui`) | **PASS**, 22 passed / 0 failed | `cargo test --offline --locked -p solidcraft-ui-egui --lib account::` |
| English/complete-catalog coverage for new messages | **PASS** | `... i18n::tests::every_complete_catalog_carries_every_english_message --exact` |
| Focused clippy (`-D warnings`) for native, UI and desktop crates | **PASS** | `cargo clippy ... --all-targets -- -D warnings` |
| `cargo xtask lists` | **PASS** | pending locale file folds and catalogs stay sorted |
| `cargo xtask layers` | **PASS**, no layering violations | native-only closure guard passed |
| `solidcraft-ui-egui` wasm library check | **PASS**, 0 errors (9 pre-existing warnings) | `cargo check ... --target wasm32-unknown-unknown --lib` |
| Full `solidcraft-ui-egui` library suite | **FAILED**, 5091 passed / 58 failed | 56 `TexturesDelta` drop panics and 2 agent-discovery child-process failures; two representative failures reproduced at base `ad9ab83f8` (see Limits) |
| Full desktop app test binary | **FAILED**, 169 passed / 2 failed | both are the same pre-existing `TexturesDelta` drop panic signature |
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
  prove the attempt fence, explicit browser action, verification-before-persistence and UI states,
  not a live provider, server, staging, keychain or production authority.
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
- Existing production authority remains disabled: no reviewed WorkOS client id or product origin
  ships, no installation bootstrap exists, and there is no real keychain validation,
  refresh/revocation handling or settlement authority.

## Remaining gates

1. Reviewed WorkOS client id and product HTTPS origin in desktop configuration, then a real
   device-code completion with the exact configured origin.
2. Installation bootstrap and startup tombstone recovery for a real desktop install.
3. Linux direct-binary packaging and the full account flow matrix on a real install.
4. Windows MSI packaging, Credential Manager behavior and durability/ACL checks.
5. macOS package and account flow in an authorized environment, or keep the release gate RED.
6. Real OS keychain integration and GUI account-menu/Chat/Review prompt verification against a
   clean baseline revision, including no auto browser launch.
7. Measured stop budgets for every controlled entry, including waits, queued work and late
   results.
8. Staging/live account authority, settlement, restart recovery and revocation revalidation;
   none is claimed here.
