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
3. It activates a product session only when the response is `200` and the frozen status shape
   proves an active account: valid `acct_…` id, `active` status, `email_verified`, supported
   client and protocol, `eligible`, no reasons, and a coherent balance/held/available snapshot.
4. The stored product bearer is the exact provider token, because the server's documented scheme
   verifies that token on every product request. The client does not parse, refresh or mint a
   second token.

The exchange is explicit:

- `CredentialSession::complete_provider_login` still never activates a product session.
- `BlockedProductExchange` remains the default for callers that do not opt in.
- The desktop and CLI configure no product service origin and no shipped provider sign-in, so
  `sign_in` still refuses and no production billing, commit or MCP authority is installed.
- No token is sent to any origin other than the caller-configured `WireConnection`.

## Fail-closed cases covered by local tests

- provider login alone, and the default blocked exchange;
- non-`200` statuses, including `401`/`403` and server errors;
- disabled/deleted accounts and unverified email;
- wrong, unsupported or ineligible client/protocol metadata and non-empty reasons;
- missing, malformed, over-long or non-`acct_` account identifiers;
- incoherent balance/held/available values;
- malformed and over-sized response bodies, timeout and redirect transport failures;
- request shape: exact configured origin, contract headers, provider bearer, no body.

Tests live in `crates/account-native/src/session_verification_tests.rs` and use a scripted
transport. They perform no network request and use no real credential.

## Explicit non-goals

This checkpoint does not deliver:

- a shipped product service origin, WorkOS client id, provider sign-in command or browser flow;
- token expiry/refresh handling, revocation polling or periodic revalidation;
- production billing, settlement, MCP authority or controlled execution;
- restart-time server revalidation (startup still relies on the earlier activation and the
  persisted session binding);
- real keychains, staging, Windows or wasm validation.
