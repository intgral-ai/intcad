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
  `CredentialSession::persist_verified_login_with_installation`; a provider-only or rejected result writes nothing.
- The worker's attempt id is checked before and after verification and again when the final
  write is published, so cancel, sign-out, account switch, shutdown and late replies cannot
  install a stale session. A worker that loses the generation race leaves no readable session:
  promotion first revokes the old commit marker and only writes the new marker after every staged
  value. There is no compensating-delete path to rely on. `State::poll` adopts each terminal
  snapshot once on the host logic step, never on a paint pass; while an attempt or cleanup is
  pending the host asks for a bounded repaint so code, expiry, completion and sign-out reach an
  idle window without unrelated input.
- The browser is launched only by a separate explicit "Sign in in browser" action, only at the
  HTTPS verification address from the current provider prompt and only when that address has the
  same exact configured AuthKit origin; sign-in itself never opens one. That action also requires
  platform-provenance activation: app-injected automation input fails closed. Chat and Review
  inline prompts show the same code, address, browser and cancel controls, with expiry/retry, so
  a refused surface can finish the flow without replaying the original operation.
- The desktop ships no reviewed WorkOS client id, AuthKit verification origin or product service
  origin. [Desktop build configuration](desktop-configuration.md) compiles in only explicit
  values through `WorkOsPublicClient::new` and `WireConnection::new`; with any missing or
  malformed value, `sign_in` refuses before any request. No runtime setting, environment
  heuristic, token field, response value or arbitrary URL is accepted.
- The CLI has no shipped provider sign-in. With `INTCAD_PRODUCT_HTTPS_ORIGIN` configured at
  build time, controlled headless MCP and DFM can use the existing desktop credentials and
  shared lock. If `INTCAD_WORKOS_PUBLIC_CLIENT_ID` is also set, the CLI uses the desktop
  identity journal for that client ID and origin. It verifies the lease before admission or
  controlled execution; rotation replaces the bearer, not the stable scope or operation proof.
  If the client ID is omitted, the CLI can use the existing product bearer if the server still
  accepts it, but cannot refresh it. If the configured identity cannot be opened or verified,
  controlled work refuses rather than falling back to a stale bearer.
- No token is sent to any origin other than the caller-configured `WireConnection`.

## Durable activation boundary

The credential store has individual key writes, not a multi-key transaction. The client therefore
does not treat any staged or partial value as a session:

1. A desktop sign-in worker holds its promotion lock from the first staging write through the
   commit. Cancel and sign-out try-lock that gate. If promotion is in progress, the UI action is
   refused without waiting and reports that the sign-in is completing; it never claims a
   cancellation or sign-out that did not own the durable boundary.
2. Staged keys under `login/stage/…` are written first, then the provider, account and product
   keys, and the validated installation pair. None of these are read as authority.
3. The single `login/commit` key is written last with the attempt generation and account. Its
   presence, together with the matching account/product/installation values and an absent
   sign-out marker, is the commit/linearization point. Every reader — product bearer, installation
   credential, scope, settlement/execution proofs, bound account and display-only identity —
   requires it.
4. A crash or storage failure before the commit leaves staged/partial values that every reader
   ignores. Clearing authority first writes `login/commit` to the revoked value, so even a
   partially deleted old session is non-restorable once that write succeeds. After a failed
   commit, credential reads and ordinary writes stay closed. A fresh verified attempt can retry
   by writing the revoked value again, then committing a new session.
5. An accepted sign-out revokes authority in process immediately, then runs one async worker that
   writes the tombstone and deletes under its own generation. The first action view is pending;
   sign-in is hidden while cleanup is pending or failed. A failure offers a sign-out retry.
   Repeated requests while pending are refused without a second worker. Sign-out is successful
   only after the background worker confirms cleanup. Abrupt termination before the durable
   tombstone is written may leave the prior committed identity restorable. During promotion,
   sign-out is refused rather than queued.
6. A completed commit is a completed login; cancel after it is refused and sign-out is the
   lifecycle transition. Shutdown waits at the exit gateway for an in-progress promotion to
   finish before invalidating the generation, so a serialized commit winner is not invalidated
   after the fact. Normal exit joins background account workers and can wait on the OS keyring;
   there is no bounded exit guarantee. Real OS keyring latency and destructive crash acceptance
   are unverified. The process does not claim atomic multi-key keyring behavior beyond this
   single-key commit boundary.

The production desktop path uses `persist_verified_login_with_installation` with its attempt
generation and the installation pair returned by the configured product service. The older
`activate_product_session` method remains a test/composition seam and writes the same commit
marker; no shipped production caller uses it. Any future production caller must use the staged
generation path rather than promote provider keys through a marker-exempt path.

This is a conservative design and has a UX cost: a cancel or sign-out clicked while the final
keyring promotion runs is refused with a retry/sign-out message instead of waiting or pretending
to succeed. A verification that succeeds after cancellation is not activated; a verification that
already committed finishes as a normal login. An existing credential session written before this
commit marker was introduced is treated as non-restorable and requires a fresh sign-in.

## Restart presentation and settlement


`AccountRuntime::startup` treats persisted credentials as a candidate, not an active session. A
candidate needs the complete stored pair: a provable account/installation/session scope, the exact
product bearer and the installation credential. A missing installation credential, a null or
read-only store, a durable sign-out marker or an unreadable credential state yields no candidate
and no signed-in view.

Before promoting the signed-in view, the desktop performs one bounded `GET /v1/status` against the
configured product origin with the captured exact bearer. Startup itself does no network read: it
reads only local facts before the window exists and leaves the cached view signed out/pending.
After the window is created, `DesktopAccountPort` starts one bounded native worker for that single
check, so window creation and logic ticks do not wait on HTTP. The host samples the worker's
waiting state before adopting the cached view on each logic tick; a completion that lands between
those two reads therefore still schedules one more bounded wake instead of leaving a stale
signed-out view. Restart applies the same consumed activation predicate as fresh login: active and
email-verified account, exact supported protocol, supported client, `eligible` true with empty
`reasons`, and coherent balance fields. The reply's account id must equal the captured local
account. After the request, the worker re-reads the scope, bearer, installation credential and
revocation generation; any change discards the result. An explicit sign-in, sign-out, account
switch or GUI shutdown retires the check immediately, marks its shared result done and drops a
late reply. There is no installation bootstrap or replacement, no browser opening, no operation
re-execution and no automatic token refresh on this path.

The generic `AccountService::status` decoder remains the broader quota/status snapshot. Restart
narrows only its own presentation decision; it does not narrow the generic status contract. A
successful restart read proves only that the server accepted the exact bearer for this account
under those activation conditions. It does not prove that the stored installation credential is
still server-valid, and it installs no billing, commit, MCP or controlled-execution authority.
Those remain absent until a separately reviewed account service and installation-validity path
exist.

For a verified unchanged binding, the restart worker can retry a recorded settlement using the
original operation ID under a per-scope process lock. It never re-executes the operation.

## Fail-closed cases covered by local tests

- provider login alone, and the default blocked exchange;
- non-`200` statuses, including `401`/`403` and server errors;
- disabled/deleted accounts and unverified email;
- wrong, unsupported or ineligible client/protocol metadata and non-empty reasons;
- missing `account/id`, a non-`acct_` id, `acct_` with a suffix shorter than 8 or longer than 64
  characters, and a disallowed suffix character;
- incoherent balance/held/available values;
- malformed and over-sized response bodies, timeout and redirect transport failures;
- request shape: exact configured origin, contract headers, provider bearer, no body;
- restart activation parity: unverified email, ineligible client, unsupported protocol metadata,
  wrong exact protocol version, account mismatch, missing installation credential, timeout/refusal
  and a reply that arrives after the local generation changed;
- restart replay/gesture guards: exactly one `GET /v1/status`, no `POST /v1/installations`, no
  browser opener call and no presentation or authority from the local candidate alone;
- restart worker boundaries: startup sends no request before the window; a completion between the
  cached-view read and the wake sample still schedules another wake; retirement by explicit
  sign-in, sign-out, switch or shutdown marks the shared result done immediately and drops a late
  reply.
- browser launch from app-injected automation input, and an unreadable startup sign-out state.

The positive fixtures also cover a coherent zero balance and a positive held balance. They prove
only that the consumed arithmetic is accepted; they do not establish credit authority.

Fresh-login tests live in `crates/account-native/src/session_verification_tests.rs`; the shared
activation predicate is exercised by restart tests in `crates/account-native/src/http_tests.rs`
and desktop restart tests in `apps/solidcraft/src/account.rs` (the latter in the existing account
test module). They use scripted transports. They perform no network request and use no real
credential. This note does not claim live, staging or production verification.

## Before production activation

Before production, verify the trusted build-time origins, hosted device flow, same-installation
recovery, exactly-once settlement, revocation and OS keychain behavior against the staging
service. A readable sign-out marker offers cleanup only; an unreadable marker offers neither
sign-in nor cleanup. No production authority is installed by this checkpoint.

## Explicit non-goals

This checkpoint does not deliver:

- a shipped production service origin or WorkOS client id;
- periodic token refresh or account revocation polling;
- production billing, settlement, MCP authority or controlled execution;
- a claim that a successful restart status read proves installation-server validity;
- real keychains, staging, Windows or wasm validation.
