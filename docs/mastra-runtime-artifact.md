# Mastra runtime artifact inventory

**Status: reproducible evidence only — the artifact is NOT approved.** Four
required `@mastra/core` files carry Mastra Enterprise License headers and
`src/ee/index.ts` source-map provenance, so the contract's enterprise exclusion is
blocked on legal review. `policy.approvals` is empty and `check:deps` reports
`unapproved`. No Vector Studio source was imported and no legal sign-off is
claimed.

## Recorded values

| Item | Value |
|---|---|
| Target | `x86_64-unknown-linux-gnu` (evidence only) |
| Runtime package | `@intgral-ai/solidcraft-mastra-runtime@0.1.0` |
| Node | `22.23.4`, official linux-x64 archive `sha256:1805bcd35e6815559bd2eb97db438d3fdb0eb3a5e6a977b7c1badca76337e9ce` |
| Node executable in artifact | `sha256:94efd062ed066c1a0b332e18249859c905e920eeada5df55d24c99b919629a90` |
| npm | `11.19.0`, archive `sha256:31e9770f7dc71119a58509353b27917557aaf0ac9b5ef1a0465ee7d8ec67ae75` |
| Lockfile | `package-lock.json`, `sha256:fd09e98344a3242c76056411a2dcca580c60120cc17bffeb6906ef99b34f48b0` |
| Contract bundle | `schema/mastra-runtime/v1/manifest.json`, `sha256:f5e9ed0238da5b9f1ce5cf70bff4a23ed7134476cab5c43b717134c4a282662a` |
| Built entry point | `dist/src/main.js`, `sha256:c8d8c6599ea4288c16bcfd21739dcf6a84a79c9cf04ca858f64f818bf53a265d` |
| Production dependencies | **151** |
| License evidence entries | **155** (151 packages + runtime + Node + TypeScript build tool + enterprise-content review) |
| Artifact files | **11,620** = 11,618 outputs + `inventory.json` + `runtime.json` |
| Artifact bytes | **291,562,109** |
| Inventory bytes | **12,470,929** |
| Inventory digest | `sha256:24e5bf24956bd03e7f0006e092f68511a3194345d021350f8d4901a1fe8d28be` |
| `policy.approvals` | `[]` — unapproved |
| Policy digest | `sha256:cc65d32fec291b31f2079beeaac877ca9208cf97051693962b008eb02ab32fb3` |
| `LICENSES.txt` | `sha256:0b8b47113c30f9b3235519314bbae9e2bbf9559d530e5276149e71c5c318beba` |
| `NOTICE.txt` | `sha256:1fa7df190165c883dadb04e2758a1463f480bee38c9f2ff0bdf6dbd82e2c09ec` |

Artifact shape:

```text
node
dist/src/**                 34 files
node_modules/**             production closure, `ee/` paths pruned
schema/mastra-runtime/v1/**  1,103 files
package.json, README.md, DEPENDENCIES.md
licenses/LICENSES.txt
notices/NOTICE.txt
inventory.json
runtime.json
```

## Enterprise-content blocker

- `find target/mastra-runtime-artifact -path '*/ee/*' | wc -l` -> **0**. Path-level
  `ee/` pruning works.
- Four shipped files are still enterprise-license content:
  - `node_modules/@mastra/core/dist/ee-iTj2bRDt-By_B9ngH.js`
  - `node_modules/@mastra/core/dist/ee-iTj2bRDt-By_B9ngH.js.map`
  - `node_modules/@mastra/core/dist/ee-iTj2bRDt-D3-QLGDF.cjs`
  - `node_modules/@mastra/core/dist/ee-iTj2bRDt-D3-QLGDF.cjs.map`
- Their headers say `@license Mastra Enterprise License`; the source maps name
  `src/ee/index.ts`. The current public `@mastra/core/agent` entry imports them,
  so removing them makes the runtime fail to load.
- The inventory records them under `lic-mastra-enterprise` with
  `verdict: "needs-human"` and expression `LicenseRef-Mastra-Enterprise` — not
  Apache-2.0. `LICENSES.txt` and `NOTICE.txt` state the same.
- Four packages publish no separate licence file. Their entries are also
  `needs-human`: `@ai-sdk/provider-utils` 4.0.63/5.0.72,
  `@standard-community/standard-json` 0.3.7 and
  `@standard-community/standard-openapi` 0.2.11.
- The Rust artifact gate performs a content scan of shipped `@mastra/core`
  `.js`/`.cjs`/`.mjs`/`.map` files for `Mastra Enterprise License`,
  `@license Mastra Enterprise License` and `src/ee/`; a marker fails with
  `ENTERPRISE_INPUT`. The npm generator also fails on any unexpected `ee/` path
  outside the known pruned Mastra enterprise directories, instead of silently
  dropping an injected file. Because no approval exists, the gate already fails
  closed with `UNAPPROVED_ARTIFACT`.

## Reproduce

```sh
cd <repo>
npm ci --no-audit --no-fund
npm run build -w @intgral-ai/solidcraft-mastra-runtime
SOLIDCRAFT_NODE_BIN=path/to/official/node npm run check:deps -w @intgral-ai/solidcraft-mastra-runtime
```

`check:deps` rebuilds the artifact under `target/mastra-runtime-artifact` and the
input tree under `target/mastra-runtime-inputs`, compares every digest with
`packaging/mastra/inventory.json`, and **exits non-zero with `status:
"unapproved"`** while the enterprise content/legal review is open.

Artifact admission is expected to fail closed:

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
# -> UNAPPROVED_ARTIFACT (no approval). If an approval is added, the content
#    scan fails ENTERPRISE_INPUT for the four dist/ee-* files.
```

Generate mode (only when intentionally refreshing evidence):

```sh
node packages/mastra-runtime/scripts/runtime-artifact.mjs --generate \
  --node-bin path/to/official/node \
  --archive-digests path/to/tarball-digests.json
```

## Still missing for #15 / #14

- Legal review of the four Mastra Enterprise License files, or a different
  `@mastra/core` build/entry that does not import them. Until then the artifact is
  blocked and `policy.approvals` stays empty.
- Legal review of the four packages that publish no separate licence file.
- Named human sign-off on the five Vector Studio source records (0 verified:
  3 needs-human, 2 rejected). No source is imported.
- #14 installed-package, signature, non-inclusion and clean-machine evidence.
- #17 trusted-checkout binding and CI evidence retention.
- macOS/Windows inventories and Node binaries.
