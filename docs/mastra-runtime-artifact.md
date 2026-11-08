# Mastra runtime artifact inventory

**Status:** one reviewed Linux x86_64 inventory. Not an installed-package or
signature approval; #14 still owns installed evidence. No Vector Studio source
was imported.

Source revision for the recorded run: `bbe21bb65692b95538c5757278ceeebd1e24ff98`.
The `runtime.json` receipt is generated per build from the current revision.

## Reviewed values

| Item | Value |
|---|---|
| Target | `x86_64-unknown-linux-gnu` |
| Runtime package | `@intgral-ai/solidcraft-mastra-runtime@0.1.0` |
| Node | `22.23.4`, official linux-x64 archive `sha256:1805bcd35e6815559bd2eb97db438d3fdb0eb3a5e6a977b7c1badca76337e9ce` |
| Node executable in artifact | `sha256:94efd062ed066c1a0b332e18249859c905e920eeada5df55d24c99b919629a90` |
| npm | `11.19.0`, archive `sha256:31e9770f7dc71119a58509353b27917557aaf0ac9b5ef1a0465ee7d8ec67ae75` |
| Lockfile | `package-lock.json`, `sha256:fd09e98344a3242c76056411a2dcca580c60120cc17bffeb6906ef99b34f48b0` |
| Contract bundle | `schema/mastra-runtime/v1/manifest.json`, `sha256:f5e9ed0238da5b9f1ce5cf70bff4a23ed7134476cab5c43b717134c4a282662a` |
| Built entry point | `dist/src/main.js`, `sha256:c8d8c6599ea4288c16bcfd21739dcf6a84a79c9cf04ca858f64f818bf53a265d` |
| Production dependencies | **151** |
| License evidence entries | **154** (151 packages + runtime + Node + TypeScript build tool) |
| Artifact files | **11,620** = 11,618 outputs + `inventory.json` + `runtime.json` |
| Artifact bytes | **291,522,774** |
| Inventory bytes | **12,449,280** |
| Inventory digest | `sha256:583b1e52fd20361a0ec7104287c98209d2410ef05a99f33df5dd2f02576d537e` |
| Policy digest | `sha256:1665f837556251213fa319f54fee337f8d6dd1e6e6148eb6e83925eebfdc7ff4` |
| LICENSES.txt | `sha256:d989d4aebd5caf64221f1f729bfeddb23027f26192aad82942dca5e3c59a3fa5` |
| NOTICE.txt | `sha256:c0de78e11fc891f2cc1d8f3f3f534e3586c0e98e1fa027bf15820cbfbf77a302` |

Artifact shape:

```text
node
dist/src/**                 34 files
node_modules/**             10,475 files (production closure, `ee/` pruned)
schema/mastra-runtime/v1/**  1,103 files
package.json, README.md, DEPENDENCIES.md
licenses/LICENSES.txt
notices/NOTICE.txt
inventory.json
runtime.json
```

## `ee/` exclusion proof

- `find target/mastra-runtime-artifact -path '*/ee/*' | wc -l` -> **0**.
- No path component named `ee` (case-insensitive) remains. `policy.json`
  `forbiddenPathComponents` is still exactly `["ee"]`.
- Four files named `dist/ee-*.js`/`.cjs`/`.map` remain under `@mastra/core`.
  They are outside the Mastra `LICENSE.md` `ee/` directory exclusion, and the
  published public `@mastra/core/agent` entry imports them; removing them makes
  the runtime fail to load. They are recorded explicitly in `NOTICE.txt` and are
  not a claim that enterprise features are enabled. This is a reviewed
  distribution note, not a legal sign-off.

## Reproduce

```sh
# Pinned Node/npm; use the official linux-x64 binary for the artifact.
# The check accepts it through SOLIDCRAFT_NODE_BIN and verifies its SHA-256.
cd <repo>
npm ci --no-audit --no-fund
npm run build -w @intgral-ai/solidcraft-mastra-runtime
SOLIDCRAFT_NODE_BIN=path/to/official/node npm run check:deps -w @intgral-ai/solidcraft-mastra-runtime
```

`check:deps` runs the direct-dependency/`ee/` source scan and then
`runtime-artifact.mjs --check`. The check rebuilds the artifact under
`target/mastra-runtime-artifact`, rebuilds the input tree under
`target/mastra-runtime-inputs`, compares every file and digest with
`packaging/mastra/inventory.json`, verifies the policy approval, and rejects any
`ee/` path. It is offline after `npm ci`; the Node binary is supplied, not
downloaded.

Generate mode (only when intentionally reviewing a new inventory):

```sh
node packages/mastra-runtime/scripts/runtime-artifact.mjs --generate \
  --node-bin path/to/official/node \
  --archive-digests path/to/tarball-digests.json
```

Artifact admission:

```sh
cargo xtask runtime-artifact --mode artifact \
  --tree target/mastra-runtime-artifact \
  --inputs-root target/mastra-runtime-inputs \
  --approval mastra-runtime-linux-x64-v1 \
  --target x86_64-unknown-linux-gnu \
  --expected-revision "$(git rev-parse HEAD)" \
  --expected-contract-digest "sha256:$(sha256sum schema/mastra-runtime/v1/manifest.json | cut -d' ' -f1)" \
  --expected-policy-digest "sha256:$(sha256sum packaging/mastra/policy.json | cut -d' ' -f1)" \
  --expected-sources-digest "sha256:$(sha256sum packaging/mastra/source-records.json | cut -d' ' -f1)" \
  --advertised true
```

## Still missing for #15 / #14

- Named human sign-off on the five Vector Studio source rights records. They stay
  `needs-human`/`rejected`; no source is imported.
- #14 installed-package, signature, non-inclusion and clean-machine evidence for
  the packaged Node/runtime.
- #17 trusted-checkout binding and CI evidence retention for the artifact job.
- A few packages publish no separate licence file; `LICENSES.txt` records their
  SPDX expression and source but not a per-package copyright text. That needs
  legal review, together with the retained `dist/ee-*.js` note above.
- macOS/Windows inventories and the corresponding Node binaries. Only
  `x86_64-unknown-linux-gnu` is approved.
