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
- The desktop starts the documented WorkOS device flow only from an explicit account action. The
  worker leaves the window thread, shows the returned code, and opens the verification page only
  when the user explicitly asks for the browser action.
- The desktop trusts the verification page only when its normalized HTTPS origin equals the
  configured AuthKit origin, and sends the provider token only to the configured HTTPS product
  base. Missing or malformed values leave sign-in unavailable; no address is baked in.
- A completed device flow is recognized only after the product service verifies the provider
  token and the product session is stored. Refresh reads the cached result and does not replay the
  exchange.
- No token is sent to any origin other than the caller-configured `WireConnection`.

## Desktop orchestration

`apps/solidcraft/src/account.rs` owns one explicit sign-in worker per process:

1. `sign_in` checks the sign-out barrier, the secure store and the complete configuration before
   starting anything. A missing value returns `AUTH_UNAVAILABLE` with no request.
2. The worker calls the documented device-authorization endpoint, checks the returned
   `verification_uri` and optional complete URI against the configured AuthKit origin, and
   publishes the code for display.
3. Polling waits the server's interval, follows `slow_down` by increasing it, stops at
   `expires_in`, and checks cancellation before every request and before each credential write.
   Cancelling a worker wakes the wait immediately; the session revocation epoch also rejects a
   late write after sign-out or account switch.
4. After approval the worker stores the provider login, then calls
   `ServerVerifiedProductSession` against the configured product origin. The product-service
   refusal or transport failure leaves the product session inactive; provider success alone never
   grants authority.
5. The host reads the cached completed result. A completed view reports the account label but
   still refuses chat, billed tools and commit authority because no installation bootstrap or
   account service is wired.

Production configuration is three explicit host values, all required together:

- `INTCAD_WORKOS_CLIENT_ID`: the approved WorkOS public client id (never a secret).
- `INTCAD_AUTHKIT_ORIGIN`: the trusted HTTPS AuthKit origin, for example the hosted domain only.
- `INTCAD_PRODUCT_SERVICE_ORIGIN`: the trusted HTTPS product service base (host only).

Tests script both transports, a virtual clock and an inert browser opener. They perform no network
request, open no browser and use no real credential. Late authorization after cancellation or
account switch, untrusted origins, slow-down and deadline handling, provider success followed by a
product refusal, and verified success without a replayed exchange are covered locally.

This checkpoint does not yet:

- perform `POST /v1/installations` or persist an installation credential, so a restarted process
  has no provable `AuthenticatedHostScope` and controlled authority stays closed;
- install a reviewed account service, quota snapshot, billing or commit authority;
- render the device-code prompt or account menu in the UI (Entry 3);
- refresh, revoke or revalidate the product session after startup;
- run against a live WorkOS, AuthKit or product deployment.

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

The following are required together; none is present in this build:

- a **trusted configured product service origin**, supplied by reviewed configuration rather than
  an environment heuristic, token field or response value; the desktop reads it from one explicit
  host variable and validates it as HTTPS with no path prefix (implemented, unverified live);
- a **native HTTPS transport with redirects disabled** (the existing production `ureq` transport
  sets `https_only`, `max_redirects(0)` and platform certificates) and tests that exercise it
  without a live service (implemented; local tests use scripted transports);
- **startup tombstone recovery**: a durable sign-out marker may be cleared only after a fresh
  provider login is verified against the configured origin and the new product session is bound
  to the account/installation; a marker alone never restores authority (not implemented; the
  desktop still refuses sign-in while a marker is unconfirmed);
- **installation bootstrap**: `POST /v1/installations` with the activated product bearer and a
  provable host scope, followed by durable installation credential storage, so a later
  `AuthenticatedHostScope` can be proven (not implemented);
- token expiry/refresh handling, revocation polling or periodic revalidation (not implemented).
- live WorkOS/AuthKit/product verification with the approved public client id and staging origins
  (not run; requires the QA configuration named above).

## Explicit non-goals

This checkpoint does not deliver:

- a baked-in product service origin, WorkOS client id or AuthKit origin (all three are required
  from the host environment and remain unset in the shipped build);
- a finished account entry UI, prompt expiry/retry presentation or account menu (Entry 3);
- installation bootstrap, token expiry/refresh handling, revocation polling or periodic revalidation;
- production billing, settlement, MCP authority or controlled execution;
- restart-time server revalidation (startup still relies on the earlier activation and the
  persisted session binding);
- real keychains, staging, Windows or wasm validation.
