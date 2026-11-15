# Vector Studio source audit — #15

**Pinned revision:** `acb5048120b53ae1b4502724b70d1283e8c9daf4`
(`intgral-ai/intgral-vector-studio`). This is the revision in
`packaging/mastra/source-records.json` and contract section 1.5.
**Result:** 0 `verified`, 3 `needs-human`, 2 `rejected`. No Vector Studio source
has been copied into this repository.

## Summary

| Record | Upstream path | SHA-256 | Bytes | License | EE | Excluded-role scan | Verdict |
|---|---|---|---|---|---|---|---|
| `agent-loop` | `apps/server/src/mastra/agent-loop/agent-loop.ts` | `38293cdd5113cc3a7f219edcf3a45b7a8157a492fbf8404e312663df5eeb4757` | 20502 | `MIT OR Apache-2.0` grant; no file header | none | database/product coupling | `needs-human` |
| `cad-generation` | `packages/ai-runtime/src/workflows/cad-generation.ts` | `8af464230b3cac444589f8739e46f829c6b64d0834341ad96eb420018c297830` | 4795 | none for the excluded role | none | script-execution | `rejected` |
| `model-runtime` | `packages/ai-runtime/src/models/model-runtime.ts` | `338f8e98bd2d8f014047271063d7854d5b54f66d73c8649a9103f7cca0aa7823` | 6402 | `MIT OR Apache-2.0` grant; no file header | none | clean | `needs-human` |
| `product-generation-runtime` | `apps/server/src/mastra/services/product-generation-runtime.ts` | `67bf03fe44c87b6f939c755c26cf548225ee8af578279dfa97d651db5e00a5b7` | 15105 | none for the excluded role | none | database/account/bootstrap | `rejected` |
| `query-proposal` | `apps/server/src/mastra/workflows/query-proposal.ts` | `56fb644bd1be14a64b27e3f7bf963cd00458326f2cdc54c5eaace4990e3f3d5e` | 46123 | `MIT OR Apache-2.0` grant; no file header | none | script prompts + product DB/account | `needs-human` |

## Exact digest commands

```bash
# Exported bytes (read-only export outside the source repository)
VECTOR_STUDIO=path/to/intgral-vector-studio
AUDIT_EXPORT=path/to/read-only-export
mkdir -p "$AUDIT_EXPORT"
git -C "$VECTOR_STUDIO" archive \
  acb5048120b53ae1b4502724b70d1283e8c9daf4 | tar -x -C "$AUDIT_EXPORT"
sha256sum "$AUDIT_EXPORT/<path>"
stat -c%s "$AUDIT_EXPORT/<path>"

# Git object bytes, bypassing any working-tree state
git -C "$VECTOR_STUDIO" show \
  acb5048120b53ae1b4502724b70d1283e8c9daf4:<path> | sha256sum
git -C "$VECTOR_STUDIO" cat-file -s \
  acb5048120b53ae1b4502724b70d1283e8c9daf4:<path>
```

Every file exists at the pinned revision. Both the export and the Git object
produced the same digest and size.

## Scan results

```bash
grep -inE "ee/|enterprise" <file>                       # no matches in all five
grep -inE "python|pyodide|child_process|spawn\(|exec\(|execFile|subprocess|\.py\b|smlibpy" <file>
grep -inE "postgres|database|bootstrap|account|quota|migration" <file>
grep -inE "copyright|spdx|licensed under|apache|mit license" <file>   # no matches in all five
```

- **`ee/`:** none of the five files imports or references `ee/` or enterprise
  modules. `packaging/mastra/policy.json` still forbids the `ee` path component.
- **Script executor:** positive in `cad-generation` (`SCRIPT` input,
  `anatomy-smlibpy`, `CadExecutionPort`) and in `query-proposal` (smlibpy prompt
  text, `/tmp/model.py`, `prepareCadScriptProposal`).
- **Account/database/bootstrap:** positive in `agent-loop` (product
  PostgreSQL/admission composition, raw SQL), `product-generation-runtime`
  (PostgreSQL, `app.*` SQL, claims, owner identity, product-run lifecycle) and
  `query-proposal` (`ConnectedStudioService` database/account seams).
- **License headers:** none in the five files. No root `LICENSE`, `COPYING` or
  `NOTICE` exists at the pinned revision, and the workspace `package.json` files
  have no `license` field. The only license basis is the
  `vector-studio-owner-declaration` grant.

## Third-party dependencies

Direct imports are permissive: `@mastra/core@1.78.0` (Apache-2.0; its `LICENSE.md`
excludes every `ee/` directory), `zod@4.6.7` (MIT) and `node:crypto` (Node.js
built-in). `@mastra/observability@1.20.1` is an Apache-2.0 package-level dependency
of `@vector-studio/ai-runtime`, not a direct import in these files. The exact
shipped artifact, transitive closure and `ee/` exclusion remain #7's locked
inventory work; this audit does not substitute for it.

`anatomy-smlibpy` declares an upstream MIT provenance
(`wheresryan22/anatomy` at `454a0ef801b6d9b905f08309d094491c7085b3dd`, `SKILL.md`
sha256 `1993151ca39afd7880a20d871aacc0ddd348090530d8135dc5ea51af28f25305`).
Retain its MIT notice only if that skill is retained in an adapter. The five
audited files contain no notice of their own to retain.

## Why each rejected path is rejected

- `cad-generation.ts` is a prepared-`SCRIPT` wrapper around `anatomy-smlibpy` and
  a remote `CadExecutionPort`. Contract sections 1.2 and 1.5 place script
  execution outside the selected migration and permit only the
  workflow/injected-executor separation as a reference.
- `product-generation-runtime.ts` is PostgreSQL/account/product-persistence and
  product-server composition. Contract section 1.5 requires replacing those with
  desktop ports and forbids importing the product-server bootstrap.

Neither path is an import candidate. Only their structural patterns may inform an
original independent adapter.

## Remaining blocker

The three non-rejected records need a **named human sign-off** for the per-file
rights/license record. The current authorization does not contain a named personal
signatory or public attestation, and no root source license was identified. Until
that sign-off lands, the records stay `needs-human` and no source is imported.
`cargo xtask runtime-artifact --mode policy` still reports valid static metadata
only; it does not approve the artifact or replace human review.
