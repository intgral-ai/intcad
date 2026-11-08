# Workflow runtime artifact inventory

**Status: reproducible evidence only — BM-1 removed `@mastra/core`; the current
inventory has no `@mastra`/`ee` path. `policy.approvals` is empty, so the artifact
remains unapproved (DQ-7). BM-3 owns the final installed-package inventory and
release policy; the historical enterprise notes below describe the pre-BM-1 tree.**

Historical pre-BM-1 status: **the artifact was NOT approved.** Four
required `@mastra/core` files carried Mastra Enterprise License headers and
`src/ee/index.ts` source-map provenance, so the contract's enterprise exclusion is
blocked on legal review. `policy.approvals` is empty. `check:deps` is evidence mode: it verifies the
reproducible inventory and reports `unapproved` with exit 0. Packaging/release
must use `check:artifact-approval` (or `runtime-artifact --mode artifact`), which
fails closed. No Vector Studio source was imported and no legal sign-off is
claimed. The packaging layout, startup checks and installed-target documentation
are in [mastra-packaging.md](mastra-packaging.md); they consume an approved
artifact and do not approve this one.

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
| Built entry point | `dist/src/main.js`, `sha256:460434466c23001cfcc134dec6c36a97750fb8679c70cc6e7804d4175bbe6f82` |
| Production dependencies | **151** |
| License evidence entries | **155** (151 packages + runtime + Node + TypeScript build tool + enterprise-content review) |
| Artifact files | **11,638** = 11,636 outputs + `inventory.json` + `runtime.json` |
| Artifact bytes | **291,695,159** |
| Inventory bytes | **12,494,478** |
| Inventory digest | `sha256:b2f34e81fb4e630b4dfaaf9e06611d245b87c11737a6471bfd24b063b262d771` |
| `policy.approvals` | `[]` — unapproved |
| Policy digest | `sha256:cc65d32fec291b31f2079beeaac877ca9208cf97051693962b008eb02ab32fb3` |
| `LICENSES.txt` | `sha256:0b8b47113c30f9b3235519314bbae9e2bbf9559d530e5276149e71c5c318beba` |
| `NOTICE.txt` | `sha256:16c7b031ff79252a4c8454b277880daa122de7538b196ce99ef91f5b0e8023d6` |

Artifact shape:

```text
node
dist/src/**                 52 files
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
`packaging/mastra/inventory.json`, and exits **0** with `status: "unapproved"`
while it is reproducible and the enterprise/missing-licence content is recorded
as `needs-human`. It exits non-zero on drift, an unrecorded dependency, a checksum
mismatch, an unexpected `ee/` path, or enterprise content not recorded as
needs-human.

`check:artifact-approval` is the release/packaging mode:

```sh
SOLIDCRAFT_NODE_BIN=path/to/official/node npm run check:artifact-approval -w @intgral-ai/solidcraft-mastra-runtime
# -> status "unapproved", exit 1 until legal review and a real approval exist
```

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
