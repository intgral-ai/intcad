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
| Lockfile | repository-root `package-lock.json`, SHA-256 `fd09e98344a3242c76056411a2dcca580c60120cc17bffeb6906ef99b34f48b0` |
| Contract bundle | `schema/mastra-runtime/v1/manifest.json`, SHA-256 `f5e9ed0238da5b9f1ce5cf70bff4a23ed7134476cab5c43b717134c4a282662a` |
| Built entry point | `dist/src/main.js` from `npm run build` |

## Direct production dependencies

| Package | Version | Licence | Source |
|---|---|---|---|
| `@mastra/core` | `1.82.0` | Apache-2.0 | `https://registry.npmjs.org/@mastra/core/-/core-1.82.0.tgz` |
| `ajv` | `8.20.0` | MIT | `https://registry.npmjs.org/ajv/-/ajv-8.20.0.tgz` |
| `zod` | `4.6.7` | MIT | `https://registry.npmjs.org/zod/-/zod-4.6.7.tgz` |

`zod` is used only to declare the Mastra workflow step schemas. It was already
present in Mastra's transitive closure; this package now declares it directly so
the lockfile records the intended version.

## Development-only dependencies

| Package | Version | Licence |
|---|---|---|
| `typescript` | `5.9.3` | Apache-2.0 |
| `@types/node` | `22.20.5` | MIT |

The test runner is the Node.js built-in `node:test` module. There is no separate
test-runner package.

## Mastra enterprise-module boundary

`@mastra/core` 1.82.0 declares Apache-2.0 in its `package.json`, and its
`LICENSE.md` excludes every `ee/` directory from that grant. The package tarball
does contain enterprise directories (`agent-builder/ee`, `auth/ee` and the
internal auth bundle), so a package-level licence string or wholesale copy of
`node_modules/@mastra/core` is not sufficient.

This runtime imports only:

```text
@mastra/core/agent
@mastra/core/workflows
```

`npm run check:deps` scans both `src/` and `dist/src/` and reports no `ee/`
import. The built artifact contains only this package's `dist/src`; the packaging
step in #14 must prune or otherwise exclude unused `@mastra/core` enterprise
files before building an installed artifact. That artifact inventory remains
#15's decision.

## Transitive closure

The repository-root `package-lock.json` is the locked transitive closure and is
committed with this package. `npm ls --omit=dev --all` prints the installed tree
after `npm ci`. The reviewed closure is now
[`packaging/mastra/inventory.json`](../../packaging/mastra/inventory.json):
151 production packages, exact versions, resolved URLs, lockfile integrity and
tarball SHA-256, with a file/digest entry for every shipped file.

## Reviewed artifact

`npm run check:deps` also runs
`packages/mastra-runtime/scripts/runtime-artifact.mjs --check`. It rebuilds the
Linux x64 artifact from `dist/src`, the production `node_modules` closure, the
protocol bundle and the pinned Node binary, then compares every digest with the
reviewed inventory and rejects any `ee/` path. See
[`docs/mastra-runtime-artifact.md`](../../docs/mastra-runtime-artifact.md) for the
recorded digests, counts and the exact artifact-admission command.

The artifact excludes every path component named `ee`. `@mastra/core` also
publishes `dist/ee-*.js` outside its `LICENSE.md` `ee/` directory exclusion; the
public agent entry imports those files, so they are retained and explicitly
recorded in `packaging/mastra/NOTICE.txt`. That note is not legal sign-off.

## Verification

```sh
cd packages/mastra-runtime
npm run check:deps
npm run verify:schema
```
