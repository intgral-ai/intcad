# Account QA evidence

This page separates the original Task 13 contract snapshot from the later Linux desktop
account-entry run. Neither run used live credentials or a staging or production account service.

## Tested revision and environment

| Item | Value |
| --- | --- |
| Client commit | `46cf63a0e9132162d1366df09a3e1ed8dc15ea77` |
| Branch | `feat/int1497-entitlement` |
| Worktree state | clean (`git status --porcelain` empty) |
| Platform | Linux host (native `x86_64` build of the client crates) |
| Package type | none; no installer or portable package was built or installed |
| Target | existing client native `target/` reused; no separate build directory created |

The server side was read only at `23ab4119ba0652f2d35cc0bda0fc35d8154063d6`, clean and unbuilt.
Contract byte evidence is in [`contract-pin.md`](contract-pin.md).

## Results

| Check | Result | Evidence |
| --- | --- | --- |
| Frozen rev3 embedded bytes match client `contracts/pin.json` | **PASS** | `contract-pin.md`; `/tmp/opencode/int1497-task13/contract-hashes.log`, `server-sha256sum.log` |
| `solidcraft-account-native` frozen `contract_vectors` (offline) | **PASS**, 1/0/0, 100 filtered out | `/tmp/opencode/int1497-task13/contract-vectors.log` |
| `solidcraft-account-native` library tests (offline) | **PASS**, 100 passed / 0 failed / 1 ignored | `/tmp/opencode/int1497-task13/native-lib.log` |
| Linux direct-binary package build and install | **NOT_RUN** | no package built in this bounded checkpoint |
| Linux login / logout / account switch / recovery / verification flows | **NOT_RUN** | no configured product service, credential or shipped sign-in flow |
| Linux headless device-code flow | **NOT_RUN** | no live provider/service or headless account exercise |
| No-dialog / no-GUI property on headless paths | **NOT_RUN** | no GUI or headless account run performed |
| Linux real keyring / credential persistence | **NOT_RUN** | scripted transports were used earlier; no OS keychain exercise here |
| Windows MSI (release or GUI-only) install | **NOT_RUN** | no Windows package or authorized Windows environment used |
| Windows Credential Manager and account flows | **NOT_RUN** | same; Windows durability/ACL behavior remains unverified |
| macOS package install and account flows | **BLOCKED** | no authorized macOS acceptance environment or package |
| GUI Review panel, credit display and compact-width access | **NOT_RUN** | GUI/visual verification was outside this checkpoint; no clean-base UI claim is made |
| Measured controlled-operation stop budgets | **NOT_RUN** | no stop-bound measurement was performed |
| Live account billing, settlement or production authority | **BLOCKED** | deliberately not implemented or configured; no live credentials or service |

## Limits of this evidence

- The scripted `contract_vectors` run proves the pinned bytes and the adapter's handling of the
  frozen vectors. It does **not** prove a live server, staging, production billing, settlement,
  installation credentials or revocation behavior.
- The library run is Linux-host evidence only. It is not a wasm, Windows, macOS, packaging or GUI
  result.
- No package, installer or artifact hash is reported because no package was built. No entry or
  capability stop bound is reported because none was measured.
- `docs/account/capability-budget.md` therefore keeps every capability **UNVERIFIED**;
  `docs/account/entry-matrix.md` keeps its intended-gate wording. This checkpoint does not fill
  either table with planned values or estimates.
- Existing production authority remains disabled: there is no shipped service origin, sign-in
  command, real keychain validation, refresh/revocation handling or settlement authority.

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

1. Full Linux acceptance and account flow matrix on a real install with a persistent credential store.
2. Windows MSI packaging, Credential Manager behavior and durability/ACL checks.
3. macOS package and account flow in an authorized environment, or keep the release gate RED.
4. Real OS keychain integration and live device-code flow without GUI dialogs on programmatic
   requests.
5. GUI Review panel and credit-layout verification against a clean baseline revision.
6. Measured stop budgets for every controlled entry, including waits, queued work and late
   results.
7. Staging/live account authority, settlement, restart recovery and revocation revalidation;
   none is claimed here.
