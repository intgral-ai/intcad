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

## Before a staging package run

**Dispatch is blocked.** The wrapper rejects higher-scope keys visible to an unbound job,
then stops before reading any raw staging values. The Actions runner can log step
environment values before validation. Do not dispatch until a reviewed source can
provide environment-scoped inputs without displaying them before validation.

Use **Staging desktop packages (nonpublishing)** (`staging-packages.yml`), not the ordinary
package or release gate workflow. Once the blocker is resolved, before an authorized dispatch:

1. Verify the `staging` GitHub environment contains the exact approved public values for all
   three keys above. The staging wrapper requires a WorkOS ID of `client_` followed by ASCII
   alphanumerics and bare HTTPS origins with no trailing slash. Check that the repository
   and any organization variables visible to it do not define any of these keys. Do not
   use secrets, production values or repository/organization fallbacks.
2. Verify the manual workflow is discoverable for dispatch (GitHub requires it to exist on the
   default branch). Verify the selected run ref points to the approved, reviewed SHA.

The build path expects one tuple for Linux x86_64, both macOS builds that form the universal
package, and Windows x64. `staging-` Actions artifacts carry build evidence with the checkout SHA,
package hashes, and the exact public tuple under `accountConfig`. This workflow does not publish
a release or provide signed distribution. Package checks are not installed-app UX, sign-in or
keychain acceptance evidence.

## Relationship to installation and authority

A verified WorkOS identity is not installation proof. Explicit sign-in verifies the product
session and requests an installation pair from the same product origin. The staged login writes
that pair before publishing the single commit marker; partial writes cannot be restored.
A readable sign-out marker offers cleanup, and an unreadable one keeps account actions closed.
After restart, a complete pair remains a candidate until the bounded product status check accepts
it. The startup worker can retry recorded settlement for the same operation without re-execution.
While that worker is pending, fresh sign-in is refused rather than replacing the credentials it
is using. Participating desktop processes with the same default data directory serialize
account-store access. A second process that cannot acquire the lock keeps ordinary CAD available
but cannot access the account store. The ledger-directory override does not move the lock.
This does not coordinate with older clients or with launches that resolve a different default
data directory (for example, different `XDG_DATA_HOME` values on Linux).
These steps do not install controlled execution, admission or commit authority.
