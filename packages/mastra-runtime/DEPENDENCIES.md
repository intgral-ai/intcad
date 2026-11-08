# Locked dependency record for `@intgral-ai/solidcraft-mastra-runtime`

This record is input for issue #15. It is not a substitute for the shipped
dependency, Node notice and artifact inventory that #15 and #14 own. Cargo's
licence check does not cover this npm/Node artifact.

## Runtime pin

| Item | Value |
|---|---|
| Node | `22.23.4` (`.nvmrc`; `engines: ">=22.23.4 <23"`) |
| npm | `11.19.0` (`packageManager`) |
| Package | `@intgral-ai/solidcraft-mastra-runtime@0.1.0`, private |
| Package licence | `MIT OR Apache-2.0` (project dual licence) |
| Lockfile | repository-root `package-lock.json`, SHA-256 `c2e05f37b9ef1a7398b3b6a926b81f579c8ac76279822c4e1863a210e42377b6` |
| Contract bundle | `schema/mastra-runtime/v1/manifest.json`, SHA-256 `f5e9ed0238da5b9f1ce5cf70bff4a23ed7134476cab5c43b717134c4a282662a` |
| Built entry point | `dist/src/main.js` from `npm run build` |

## Direct production dependencies

| Package | Version | Licence | Source |
|---|---|---|---|
| `ajv` | `8.20.0` | MIT | `https://registry.npmjs.org/ajv/-/ajv-8.20.0.tgz` |

`@mastra/core`, `zod` and the legacy `language-model.ts` adapter are no longer
dependencies. `ajv` validates the protocol schema bundle. `check:deps` and
`test/package.test.ts` fail on any `@mastra` import or `ee` path.

## Development-only dependencies

| Package | Version | Licence |
|---|---|---|
| `typescript` | `5.9.3` | Apache-2.0 |
| `@types/node` | `22.20.5` | MIT |

The test runner is the Node.js built-in `node:test` module. There is no separate
test-runner package.

## No Mastra SDK / enterprise module

`@mastra/core` and `zod` are absent from `package.json` and the lockfile. The
legacy `language-model.ts` adapter and no-bridge Agent/Workflow path were removed
by BM-1. `npm run check:deps` scans `package.json`, the lockfile, `src/` and
`dist/src/` and fails on any `@mastra` import or `ee` path. The native workflow
uses only `ajv` plus Node builtins.

The artifact inventory remains #15/BM-3's decision. `policy.approvals` stays
empty and the artifact is unapproved (DQ-7); BM-1 does not unblock distribution.

## Transitive closure

The repository-root `package-lock.json` is the locked transitive closure and is
committed with this package. `npm ls --omit=dev --all` prints the installed tree
after `npm ci`. The reviewed closure is now
[`packaging/mastra/inventory.json`](../../packaging/mastra/inventory.json):
the exact production closure, resolved URLs, lockfile integrity and tarball
SHA-256, with a file/digest entry for every shipped file. BM-3 owns the final
installed-package inventory and release policy.

## Reproducible artifact evidence, not an approval

`npm run check:deps` also runs
`packages/mastra-runtime/scripts/runtime-artifact.mjs --check` in evidence mode.
It rebuilds the Linux x64 artifact from `dist/src`, the production `node_modules`
closure, the protocol bundle and the pinned Node binary, compares every digest
with the inventory, and checks for Mastra/enterprise content. It exits 0 with
`status: "unapproved"` while the content is reproducible and explicitly recorded
as `needs-human`. Packaging/release must use `npm run check:artifact-approval`
(`--require-approved`), which exits 1 while `policy.approvals` is empty. See
[`docs/mastra-runtime-artifact.md`](../../docs/mastra-runtime-artifact.md).

The current artifact contains no `@mastra` or `ee` path; `status: "unapproved"`
is only the empty `policy.approvals` (DQ-7). BM-3/legal must audit the final
installed dependency/licence closure before any release; this is not legal
sign-off.

## Verification

```sh
cd packages/mastra-runtime
npm run check:deps
npm run verify:schema
```
