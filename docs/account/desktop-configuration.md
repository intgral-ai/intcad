# Desktop account build configuration

The desktop account entry is compiled with, not configured at runtime. A trusted release builder
supplies three values as build environment variables:

| Build value | Environment variable | Accepted form |
| --- | --- | --- |
| WorkOS public client id | `INTCAD_WORKOS_PUBLIC_CLIENT_ID` | non-empty public client id, no whitespace or control characters |
| AuthKit verification origin | `INTCAD_AUTHKIT_VERIFICATION_ORIGIN` | bare HTTPS origin (`https://host[:port]`) |
| Product service origin | `INTCAD_PRODUCT_HTTPS_ORIGIN` | bare HTTPS origin (`https://host[:port]`) |

On macOS, the build script writes one compiled account record that the desktop reads.
On other platforms, `apps/solidcraft/src/account.rs` reads the values with `option_env!`.
In either case they are fixed in the binary.
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

## Production packages

The formal release gate (`release-gate.yml`) passes
`production_account_config: ${{ vars.INTCAD_PRODUCTION_ACCOUNT_CONFIG }}` to
`release.yml`. The variable is a protected repository variable holding one JSON object with
exactly the three keys above and the same accepted forms as the staging tuple. It lives outside
this repository's files and is never written to logs: `verify-production` prints only the
tuple's SHA-256.

The build fails closed instead of producing an anonymous candidate:

- the release gate is refused when `production_account_config` is empty;
- `release.yml` refuses production values from any caller other than the release gate, and
  refuses a build that supplies both staging and production values;
- the values are supplied before compile exactly as in staging.

Package evidence (`build.json`, `accountConfig`) records
`{"source":"environment:production","sourceRevision":"<checkout SHA>","tupleSha256":"<SHA>"}`
instead of the values. The checkout SHA and the tuple SHA together answer, for any candidate,
which account its bytes embed.

## Verifying a candidate binary

A reviewer can run the package check on the extracted desktop executable from the exact source
checkout. Supply the reviewed JSON snapshot in `STAGING_ACCOUNT_CONFIG` or
`PRODUCTION_ACCOUNT_CONFIG`, not in the application's build variables. For example, for staging:

```sh
GITHUB_SHA="$(git rev-parse HEAD)" \
STAGING_ACCOUNT_CONFIG="$(git show HEAD:.github/config/intcad-staging-account.json)" \
python3 -I scripts/ci/verify_embedded_account.py --macho-universal /path/to/Intcad.app/Contents/MacOS/Intcad
```

Omit `--macho-universal` for Linux and Windows. On macOS the check requires the complete account
record in both mounted slices before `build.json` is written. Compare the reported tuple SHA-256
with the package evidence. `scripts/ci/staging_account_config.py verify-production` reports the
expected production digest. Neither command launches the app, contacts an account service, or
prints the configured values. The package check does not verify sign-in.

## Before a staging package run

Use **Staging desktop packages (nonpublishing)** (`staging-packages.yml`), not the ordinary
package or release gate workflow. Before an authorized dispatch:

1. Review the exact public staging tuple in `.github/config/intcad-staging-account.json`
   at the approved SHA. The staging wrapper requires a WorkOS ID of `client_` followed
   by ASCII alphanumerics and bare HTTPS origins with no trailing slash. Never put secrets
   or production settings in that file. Rotate the tuple through a new reviewed commit.
2. Verify the manual workflow is discoverable for dispatch (GitHub requires it to exist on the
   default branch). Verify the selected run ref points to the approved, reviewed SHA and
   the `staging` environment gate remains in place. No GitHub variables are read at runtime.

The build path expects one tuple for Linux x86_64, both macOS builds that form the universal
package, and Windows x64. `staging-` Actions artifacts carry build evidence with the checkout SHA,
package hashes, and the exact public tuple, path and source SHA under `accountConfig`.
This workflow does not publish a release or provide signed distribution. Package checks are not
installed-app UX, sign-in or keychain acceptance evidence.

## Relationship to installation and authority

A verified WorkOS identity is not installation proof. Explicit sign-in verifies the product
session and requests an installation pair from the same product origin. The staged login writes
that pair before publishing the single commit marker; partial writes cannot be restored.
A readable sign-out marker offers cleanup, and an unreadable one keeps account actions closed.
After restart, a complete pair remains a candidate until the bounded product status check accepts
it. The startup worker can retry recorded settlement for the same operation without re-execution.
While that worker is pending, fresh sign-in is refused rather than replacing the credentials it
is using. The desktop and CLI use the same OS-resolved per-user account directory for the lock
and ledger. `HOME`, `XDG_DATA_HOME`, `APPDATA`, and settings overrides do not change it.
A process that cannot acquire the lock keeps ordinary CAD available but cannot access the
account store. The CLI does not create the desktop's lock or ledger directory. Older clients
using a different data directory do not participate in the shared lock.
These steps do not install controlled execution, admission or commit authority.
