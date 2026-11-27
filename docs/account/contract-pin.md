# Frozen account contract pin evidence

This note records the client-side byte check against the frozen rev3 contract at the pinned
server revision. It does not restate, reinterpret, copy or replace the server-owned contract.
`contracts/pin.json` remains the only client pin; this note is evidence that the exact reviewed
bytes were read successfully at Task 13.

## Revision

| Side | Revision | State |
| --- | --- | --- |
| Client | `46cf63a0e9132162d1366df09a3e1ed8dc15ea77` (`fix(xtask): keep unknown target conjuncts in the closure guard`) | clean worktree |
| Server (read-only) | `23ab4119ba0652f2d35cc0bda0fc35d8154063d6` (`docs: record runtime policy local acceptance`) | clean worktree |

- Frozen artifact: rev3, Linear document slug `3c7c6dfefe52`.
- Protocol: `intcad-auth/1.0`; OpenAPI `info.version` `1.0.0`.
- Server path read: `/home/hyjing/.local/share/opencode/worktree/7a77c8/int-1497-server/contracts/v1`.
- The server worktree was read, copied out to a temporary staging directory and **not modified or
  built**. Decoding below uses the client's existing native target.

## Verified bytes

The copies in `/tmp/opencode/int1497-task13/contract-v1` were renamed to the client test's
`embedded_*` layout (`embedded_manifest.json`, `embedded_openapi.json`,
`embedded_fixtures.json`). Renaming does not alter bytes.

| Artifact | Server file | Bytes | SHA-256 | Matches `contracts/pin.json` |
| --- | --- | ---: | --- | --- |
| Manifest | `contracts/v1/manifest.json` | 1,405 | `f4d677dd066429854eac45e7776f7e2611845db9c174e26833704af5faf6b45e` | PASS |
| OpenAPI | `contracts/v1/openapi.json` | 47,822 | `20f32b8cde8141220489cbd0db4dafd42a01a1d14d9ad9c20078807c5c8afcb8` | PASS |
| Fixtures | `contracts/v1/fixtures.json` | 176,410 | `ee1d8dde51fac40debf95d1bc29555b787adfa60175901f98be2b97c73529b2b` | PASS |

The same three embedded hashes are recorded in `contracts/pin.json` (FROZEN). The client adapter
also checks the canonical hashes, versions, vector counts and support-matrix counts during load;
the vector run below passed those internal checks as part of `contract_vectors`.

Logs:

- `/tmp/opencode/int1497-task13/contract-hashes.log` — SHA-256 values of the staged copies.
- `/tmp/opencode/int1497-task13/server-sha256sum.log` — `sha256sum -c SHA256SUMS` in the
  read-only server tree: all three files `OK`.
- `/tmp/opencode/int1497-task13/contract-vectors.log` — client run at the client revision above.

## Client consumption result

```bash
INTCAD_CONTRACT_VECTORS=/tmp/opencode/int1497-task13/contract-v1 \
  CARGO_NET_OFFLINE=true \
  cargo test --offline --locked -p solidcraft-account-native --lib contract_vectors -- --include-ignored
```

Result: **PASS**, `1 passed; 0 failed; 0 ignored; 100 filtered out`. The test verifies the exact
artifacts against the pin and runs every applicable frozen vector through the scripted adapter.
The runner checks for exactly 52 applicable vectors (56 cases) and 16 explicitly listed vectors
outside the client adapter's responsibilities. The bundle's four support vectors are not that
exclusion list; the runner also checks the permission, race and rollback matrix counts.

For context only, the normal library run at the same revision also passed:
`100 passed; 0 failed; 1 ignored` (`/tmp/opencode/int1497-task13/native-lib.log`). The one ignored
test is `contract_vectors`, which requires external artifacts and is exercised above.

## Gate meaning

- **Contract pin gate: PASS.** Both repositories agree on the same frozen rev3 embedded bytes.
- **Client vector gate: PASS for the scripted contract cases.** This is not a live server,
  staging, production, platform-package or GUI result.
- **Integration/release gate remains RED.** Platform account QA, live account authority,
  measured stop budgets and production settlement are not verified here; see
  [`qa-evidence.md`](qa-evidence.md).
