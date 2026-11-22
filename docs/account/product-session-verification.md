# Server-verified product session activation

This note records the client-side seam for the documented WorkOS session scheme. It is local
evidence only: no live service, credential, deployment or production authority is exercised.

## Server evidence read

- Read-only server worktree: `/home/hyjing/.local/share/opencode/worktree/7a77c8/int-1497-server`.
- Pinned revision: `23ab411` (`docs: record runtime policy local acceptance`).
- The worktree was inspected, not edited or built.

Relevant server statements at that revision:

- `src/auth/mod.rs:3-5` says the only accepted product credential is a WorkOS session access
  token verified against WorkOS's published JWKS and user-management API. There is no anonymous
  fallback and no token trusted on shape alone.
- `src/auth/workos.rs` verifies the RS256 signature, signed claims, issuer/client id, identifiers,
  session markers and a freshly read WorkOS user record, then returns `SessionIdentity`.
- `src/http/mod.rs` (`verified_product_session`) calls `identity.verify_session(token)`; the
  status route (`src/http/status.rs`) uses that path before returning `GET /v1/status`.
- `README.md` documents that `GET /v1/status` takes the product-session bearer token with
  `X-Client-Version` and `X-Protocol-Version`, and that account mapping uses the verified WorkOS
  `sub`. The first valid request grants the initial credits and returns a same-transaction
  account/credit snapshot.

The client freeze remains `contracts/pin.json` (`intcad-auth/1.0`, rev3, OpenAPI `1.0.0`). This
document does not copy, restate or reinterpret the server contract.

## Client checkpoint

`solidcraft_account_native::ServerVerifiedProductSession` implements the existing typed
`ProductSessionExchange` seam:

1. It takes an already validated HTTPS `WireConnection` and an injectable `Transport`.
2. It sends one `GET /v1/status` with the provider access token as its bearer, the configured
   client version, the frozen protocol version and no body.
3. It activates a product session only when the response is `200` and the **consumed** status
   fields pass these checks:
   - `/account/id` matches the frozen rev3 grammar `^acct_[A-Za-z0-9_-]{8,64}$`;
   - `/account/status` is `active` and `/account/email_verified` is `true`;
   - `/protocol/version` is `intcad-auth/1.0` and `/protocol/supported` is `true`;
   - `/client/supported` is `true`;
   - `eligible` is `true` and `reasons` is empty;
   - `balance`, `held` and `available` are present `u64` values with `held <= balance` and
     `available == balance - held`.
4. The stored product bearer is the exact provider token, because the server's documented scheme
   verifies that token on every product request. The client does not parse, refresh or mint a
   second token.

This is not a general status decoder. It deliberately ignores `/capabilities`, `/policy`,
`/snapshot_at` and every other member of the status response. In particular, `eligible` and
`reasons` are used only as local fail-closed activation preconditions: activation makes no
credit, quota, capability or execution decision, and it does not prove any controlled capability
is enabled. A later adapter must decode and enforce those fields separately.

The exchange is explicit:

- `CredentialSession::complete_provider_login` still never activates a product session.
- `BlockedProductExchange` remains the default for callers that do not opt in.
- The desktop now has an explicit sign-in worker: one UI click starts one WorkOS device-code
  attempt, a background worker polls it, and the provider session is offered to
  `ServerVerifiedProductSession` before anything is written. A verified result is persisted with
  `CredentialSession::persist_verified_login`; a provider-only or rejected result writes nothing.
- The worker's attempt id is checked before and after verification and again when the final
  write is published, so cancel, sign-out, account switch, shutdown and late replies cannot
  install a stale session. Instants a worker does write after losing the generation race remove
  that attempt's own write on its worker thread. `State::poll` adopts each terminal snapshot once
  on the host logic step, never on a paint pass.
- The browser is launched only by a separate explicit "Sign in in browser" action, only at the
  HTTPS verification address from the current provider prompt and only when that address has the
  same exact configured AuthKit origin; sign-in itself never opens one.
- The desktop ships no reviewed WorkOS client id, AuthKit verification origin or product service
  origin. [Desktop build configuration](desktop-configuration.md) compiles in only explicit
  values through `WorkOsPublicClient::new` and `WireConnection::new`; with any missing or
  malformed value, `sign_in` refuses before any request. No runtime setting, environment
  heuristic, token field, response value or arbitrary URL is accepted.
- The CLI still configures no product service origin and no shipped provider sign-in; `sign_in`
  still refuses and no production billing, commit or MCP authority is installed.
- No token is sent to any origin other than the caller-configured `WireConnection`.

## Durable activation boundary

The credential store has individual key writes, not a multi-key transaction. The client therefore
does not treat any staged or partial value as a session:

1. A desktop sign-in worker holds its promotion lock from the first staging write through the
   commit. Cancel and sign-out try-lock that gate. If promotion is in progress, the UI action is
   refused without waiting and reports that the sign-in is completing; it never claims a
   cancellation or sign-out that did not own the durable boundary.
2. Staged keys under `login/stage/…` are written first, then the provider, account and product
   keys. None of these are read as authority.
3. The single `login/commit` key is written last with the attempt generation and account. Its
   presence, together with the matching account/product/installation values and an absent
   sign-out marker, is the commit/linearization point. Every reader — product bearer, installation
   credential, scope, settlement/execution proofs, bound account and display-only identity —
   requires it.
4. A crash or storage failure before the commit leaves staged/partial values that every reader
   ignores. Clearing authority first writes `login/commit` to the revoked value, so even a
   partially deleted old session is non-restorable.
5. An accepted sign-out writes the durable sign-out marker before returning where the promotion
   gate is free. During promotion sign-out is refused. Repeated sign-out generations rely on the
   marker already owned by the earlier cleanup, so a blocked delete cannot deadlock the window.
6. A completed commit is a completed login; cancel after it is refused and sign-out is the
   lifecycle transition. The process does not claim atomic multi-key keyring behavior beyond this
   single-key commit boundary.

This is a conservative design and has a UX cost: a cancel or sign-out clicked while the final
keyring promotion runs is refused with a retry/sign-out message instead of waiting or pretending
to succeed. A verification that succeeds after cancellation is not activated; a verification that
already committed finishes as a normal login. An existing credential session written before this
commit marker was introduced is treated as non-restorable and requires a fresh sign-in.

## Fail-closed cases covered by local tests

- provider login alone, and the default blocked exchange;
- non-`200` statuses, including `401`/`403` and server errors;
- disabled/deleted accounts and unverified email;
- wrong, unsupported or ineligible client/protocol metadata and non-empty reasons;
- missing `account/id`, a non-`acct_` id, `acct_` with a suffix shorter than 8 or longer than 64
  characters, and a disallowed suffix character;
- incoherent balance/held/available values;
- malformed and over-sized response bodies, timeout and redirect transport failures;
- request shape: exact configured origin, contract headers, provider bearer, no body.

The positive fixtures also cover a coherent zero balance and a positive held balance. They prove
only that the consumed arithmetic is accepted; they do not establish credit authority.

Tests live in `crates/account-native/src/session_verification_tests.rs` and use a scripted
transport. They perform no network request and use no real credential. This note does not claim
live, staging or production verification.

## Before production activation

The following are required together; the client seam exists, but no reviewed values ship:

- a **trusted build-time product service origin, WorkOS public client id and exact AuthKit
  verification origin**, supplied by a reviewed release build rather than an environment
  heuristic, token field, runtime setting or response value;
- a **native HTTPS transport with redirects disabled** (the existing production `ureq` transport
  sets `https_only`, `max_redirects(0)` and platform certificates) and tests that exercise it
  without a live service;
- **startup tombstone recovery boundary**: a fresh provider login verified against the
  configured origin may replace the identity and clear the durable sign-out marker, but only an
  installation-bound product session may restore `AuthenticatedHostScope`; the marker alone never
  restores authority, and identity without installation proof never enables controlled work;
- **installation bootstrap**: `POST /v1/installations` with the activated product bearer and a
  provable host scope, followed by durable installation credential storage, so a later
  `AuthenticatedHostScope` can be proven;
- token expiry/refresh handling and revocation revalidation for the desktop flow. The explicit
  device-code worker and browser action are wired, but no production credential has been
  exercised.

## Explicit non-goals

This checkpoint does not deliver:

- a shipped product service origin or WorkOS client id (the explicit configuration seam exists);
- an installation bootstrap, so a fresh desktop login is not yet a provable host scope;
- token expiry/refresh handling, revocation polling or periodic revalidation;
- production billing, settlement, MCP authority or controlled execution;
- restart-time server revalidation (startup still relies on the earlier activation and the
  persisted session binding);
- real keychains, staging, Windows or wasm validation.
