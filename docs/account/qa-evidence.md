# Account QA evidence

Bounded Task 13 evidence snapshot. It records only checks that actually ran. No platform package,
live credential, GUI session, real keychain, hardware, staging or production service was used.

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

## Remaining gates

1. Linux direct-binary packaging and the full account flow matrix on a real install.
2. Windows MSI packaging, Credential Manager behavior and durability/ACL checks.
3. macOS package and account flow in an authorized environment, or keep the release gate RED.
4. Real OS keychain integration and live device-code flow without GUI dialogs on programmatic
   requests.
5. GUI Review panel and credit-layout verification against a clean baseline revision.
6. Measured stop budgets for every controlled entry, including waits, queued work and late
   results.
7. Staging/live account authority, settlement, restart recovery and revocation revalidation;
   none is claimed here.
