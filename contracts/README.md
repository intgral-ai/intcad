# Frozen account contract pin

This directory contains the client-side pin for the cross-repository account contract. It does not
contain, restate, or reinterpret server semantics. The contract itself is owned by the server
repository; this pin only records which reviewed revision the client consumes.

## Source

- Frozen revision: rev3, Linear document slug `3c7c6dfefe52`
  (document id `35734209-f350-4367-a6d4-d857c7520d82`).
- Protocol: `intcad-auth/1.0`; OpenAPI `info.version` `1.0.0`.
- `31ae2cc4a2d3` is a historical wire proposal only and is not consumed here.

The server publishes the OpenAPI document and deterministic fixtures. The three reviewed values
are the fenced JSON bodies in the rev3 document:

| Artifact | Exact (embedded) bytes | SHA-256 |
| --- | --- | --- |
| Manifest | 1,405 | `f4d677dd066429854eac45e7776f7e2611845db9c174e26833704af5faf6b45e` |
| OpenAPI | 47,822 | `20f32b8cde8141220489cbd0db4dafd42a01a1d14d9ad9c20078807c5c8afcb8` |
| Fixtures | 176,410 | `ee1d8dde51fac40debf95d1bc29555b787adfa60175901f98be2b97c73529b2b` |

The manifest's own canonical SHA-256 values are recorded in `pin.json`:
`16d732b5…` for OpenAPI and `ba9e0cb1…` for fixtures. "Embedded" means the exact compact bytes in
the document fence (no trailing newline). "Canonical" means the recursively key-sorted,
whitespace-free JSON text. Both are verified; they are not interchangeable.

## Consumption

`crates/account-native/src/contract.rs` embeds `pin.json` and reads the artifacts from the directory
named by `INTCAD_CONTRACT_VECTORS`. Accepted layouts:

- the original `doc_content.md`: the test extracts the three fenced JSON bodies itself and checks
  them byte-for-byte against the pin; or
- `embedded_manifest.json`, `embedded_openapi.json`, and `embedded_fixtures.json` already extracted
  without a trailing newline.

The artifacts are deliberately not committed here: the client keeps one pin and reads the exact
reviewed bytes instead of a second, potentially diverging copy.

## Verification states

- Normal unit runs check that `pin.json` is `FROZEN` and complete. They do not run the vectors.
- `contract_vectors` is `#[ignore]` because the artifacts are external. The release gate must run:
  `INTCAD_CONTRACT_VECTORS=<dir> cargo test -p solidcraft-account-native contract_vectors -- --include-ignored`
- Missing artifacts are not a pass: the ignored test fails when explicitly included.
- A byte, canonical-hash, version, or vector-count mismatch fails the test; Task 13 must keep the
  gate RED until the pin and artifacts agree.
- Supporting matrices (`permissions`, `races`, `rollbacks`) and the design-only vectors are
  counted and recorded as unsupported responsibilities in the test; they are not silently skipped.
