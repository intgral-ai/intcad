# solidcraft-dfm-rules

The pure DFM rule layer for the native sheet-metal checks. Facts and declared data go in; one
deterministic report comes out. It does not load OCP, read files, call a model or look at a clock.

This crate is the Rust translation of the OCP-free rule layer in `intgral-ai/INTdfm` (baseline
`2f24830fcd84c8e3047eb6643139b94c27b8c1b6`). The migration contract and its recordings live in
that repository (`docs/dfm-replay.md`, `tests/fixtures/dfm-replay/v1`).

The App side measures the facts in `solidcraft-doc::sheet_dfm` and calls this crate from
`solidcraft-engine::dfm`. The validation runner exposes it as the `dfm.sheet` check kind.

## Standalone use

```sh
cargo run -p solidcraft-dfm-rules --example check -- input.json
```

`input.json` is a `CheckInput`. The library never reads it; the example does.

## What is translated now

- the thickness gate (declared vs model, placeholder 0.02 mm);
- `sm.hole_min_diameter`, `sm.hole_to_edge`, `sm.hole_to_bend`;
- profile validation, data selection (generic / factory / standard), project overrides;
- the App state mapping, including "public values never decide" and FD-7's confirmation rule.

The Python rules stay in place as the migration baseline until both gates pass for all three
rules.

## Placement

The owner authorized publishing this branch to `intgral-ai/open-skyfall-desktop` on 2026-11-13.
The intended long-term home is still the INTdfm repository, with the App pinning a version; this
branch is the reviewable migration copy. The authorization covers the implementation, contract,
TEST-ONLY fixtures and docs, not customer STEP files, private factory data or credentials.
