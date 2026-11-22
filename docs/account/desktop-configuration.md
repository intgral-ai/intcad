# Desktop account build configuration

The desktop account entry is compiled with, not configured at runtime. A trusted release builder
supplies three values as build environment variables:

| Build value | Environment variable | Accepted form |
| --- | --- | --- |
| WorkOS public client id | `INTCAD_WORKOS_PUBLIC_CLIENT_ID` | non-empty public client id, no whitespace or control characters |
| AuthKit verification origin | `INTCAD_AUTHKIT_VERIFICATION_ORIGIN` | bare HTTPS origin (`https://host[:port]`) |
| Product service origin | `INTCAD_PRODUCT_HTTPS_ORIGIN` | bare HTTPS origin (`https://host[:port]`) |

`apps/solidcraft/src/account.rs` reads them with `option_env!`, so they are fixed in the binary.
They are not read from preferences, MCP, the control channel, a design file, a provider response
or a runtime environment lookup. There is no user-editable bearer destination.

Missing or malformed values fail closed together:

- any missing value leaves the desktop unconfigured; the explicit sign-in action refuses before
  any WorkOS or product request and the account menu shows no pending attempt;
- a client id rejected by `WorkOsPublicClient::new`, a product origin rejected by
  `WireConnection::new`, or an AuthKit origin rejected by `WireConnection::new` disables the
  whole configuration;
- the WorkOS device response's verification origin must equal the configured AuthKit origin
  exactly before the prompt is published to the UI;
- the browser launch re-checks the same exact origin, so a foreign or malformed address never
  reaches the system browser;
- the browser button accepts only a platform-provenance activation; control-channel or app
  synthetic input fails closed.

No production values are checked in. A local build, developer build or CI job without the three
variables builds an anonymous, offline desktop with the existing account refusals. The release
builder must set all three together and record which values it used in release evidence.

## Relationship to installation and authority

A verified WorkOS identity is not installation proof. Sign-in may replace a durable sign-out
marker after the configured product origin verifies the session, but only the staged activation
whose commit marker was written can be shown after restart. `AuthenticatedHostScope`, controlled
entries, billing, commit authority and Chat availability still require a reviewed installation
bootstrap plus a reviewed account service; this build installs neither. The durable commit is a
single key; partial multi-key writes are conservative and non-restorable rather than a claimed
transaction.
