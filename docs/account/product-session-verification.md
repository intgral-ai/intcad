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
- The worker's attempt id is checked before and after verification and again under the persistence
  gate, so cancel, sign-out, account switch, shutdown and late replies cannot install a stale
  session. `State::poll` adopts the worker snapshot on the host logic step, never on a paint pass.
- The browser is launched only by a separate explicit "Sign in in browser" action, only at the
  HTTPS verification address from the current provider prompt; sign-in itself never opens one.
- The desktop ships no reviewed WorkOS client id or product service origin. `DesktopAccountConfig`
  accepts only explicit values through `WorkOsPublicClient::new` and `WireConnection::new`; with
  no configuration, `sign_in` refuses before any request. No environment heuristic, token field,
  response value or arbitrary URL is accepted.
- The CLI still configures no product service origin and no shipped provider sign-in; `sign_in`
  still refuses and no production billing, commit or MCP authority is installed.
- No token is sent to any origin other than the caller-configured `WireConnection`.

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

- a **trusted configured product service origin and WorkOS public client id**, supplied by
  reviewed configuration rather than an environment heuristic, token field or response value;
- a **native HTTPS transport with redirects disabled** (the existing production `ureq` transport
  sets `https_only`, `max_redirects(0)` and platform certificates) and tests that exercise it
  without a live service;
- **startup tombstone recovery**: a durable sign-out marker may be cleared only after a fresh
  provider login is verified against the configured origin and the new product session is bound
  to the account/installation; a marker alone never restores authority;
- **installation bootstrap**: `POST /v1/installations` with the activated product bearer and a
  provable host scope, followed by durable installation credential storage, so a later
  `AuthenticatedHostScope` can be proven;
- token expiry/refresh handling, revocation revalidation and an installation bootstrap for the
  desktop flow. The explicit device-code worker and browser action are wired, but no production
  credential has been exercised.

## Explicit non-goals

This checkpoint does not deliver:

- a shipped product service origin or WorkOS client id (the explicit configuration seam exists);
- an installation bootstrap, so a fresh desktop login is not yet a provable host scope;
- token expiry/refresh handling, revocation polling or periodic revalidation;
- production billing, settlement, MCP authority or controlled execution;
- restart-time server revalidation (startup still relies on the earlier activation and the
  persisted session binding);
- real keychains, staging, Windows or wasm validation.
