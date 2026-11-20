# CEF Assistant exploration archive

The CEF exploration was cancelled because the available development and
validation resources could not support it. This branch is an archive, not a
release candidate. Keep the Wry implementation on the main development line.

The final experimental code is `95c26119478cf95715c3aa7213e13b3288bb1e1d`;
the pre-CEF Wry baseline is `96a247ccb7a64fbbd45de02dc71ea4619c5a721a`.
See [the cutover notes](cef-welding-cutover.md) for the architecture and the
[repair addendum](superpowers/specs/2026-11-19-cef-integration-repair-addendum.md)
for the contracts and acceptance criteria. Historical screenshots and
performance results are not acceptance evidence for the final revision.

## Reproduce the Linux source checks

Use Linux x86_64, Rust/Cargo 1.99.1, and an isolated checkout and Cargo target.
Populate the locked Cargo inputs and extract the stock CEF Linux x86_64
minimal distribution before going offline:

- CEF archive: `cef_binary_154.0.34+g14c5a08+chromium-154.0.8037.98_linux64_minimal.tar.bz2`
- Archive SHA-1 (historical identity):
  `15c91ad767097217b2e82ba8b933fe30b134be69`
- Expected `libcef.so` SHA-256:
  `8952fb40b6bd1d8f145a27c9f4103dac92438309768d928235c4a8913eb1dba3`
- Rust CEF bindings: `154.5.0+154.0.34`

Set `CEF_PATH` to the absolute directory containing the verified `libcef.so`
and matching `archive.json`. Do not substitute a different CEF build.

```sh
export CARGO_TARGET_DIR="$PWD/target"
export CARGO_INCREMENTAL=0
rustc --version
cargo --version
sha256sum "$CEF_PATH/libcef.so"
unshare -rn -- cargo check --locked --offline -p solidcraft --bin intcad
unshare -rn -- cargo test --locked --offline -p solidcraft-ui-egui --test scenarios s43_quit_asks_to_save
python3 experiments/cef-osr/scripts/native-density.py --self-test
```

`unshare -rn` requires working user and network namespaces; use an equivalent
offline runner if unavailable. Cargo `--offline` alone does not constrain build
scripts. Record the commit, dirty status, input hashes, raw output and exits.
`s43` must actually execute one matching test.

## Limits of the archived results

- The two Cargo commands were not launched on the final revision because
  other builds occupied the host. Earlier passes are not final-revision passes.
- The wasm check needs the `wasm32-unknown-unknown` standard library for Rust
  1.99.1. It was not available in the local pinned toolchain.
- The [historical Linux guest](../experiments/cef-osr/evidence/2026-11-19-vm/README.md)
  used llvmpipe and lacked DMA-BUF. That run showed the refusal path, not GPU
  acceptance. Any future acceptance requires an identified artifact tested in
  a suitable VM first, then the same artifact on the host.
- Native CEF ownership, safe return after initialization failure, independently
  authenticated artifact origin, and packaging acceptance remain unresolved.
  Do not enable the gated GPU path or infer release readiness from this archive.
