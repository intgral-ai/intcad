# B6 OpenPGP resolved graph evidence

Candidate: `db0b8dd350bbdffed8927845c86c5028d280cb0e`. This follows the
[production boundary audit](https://linear.app/intgral/issue/INT-1454#comment-77a625c9-2e04-47e9-8899-d56cbd97eca6).
Cargo 1.99.1 resolved an export of the candidate manifests and path crates
with `--offline --locked`. `Cargo.lock` and `deny.toml` digests were
unchanged before and after resolution. No code was compiled or executed.

## Scope and reproduction

Use the native packaging selection (`-p solidcraft -p solidcraft-cli`)
on `x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu`,
`x86_64-pc-windows-msvc`, `i686-pc-windows-msvc`, `aarch64-pc-windows-msvc`, `x86_64-apple-darwin`,
`aarch64-apple-darwin`, and `x86_64-unknown-freebsd`. Use `-p solidcraft-web`
on `wasm32-unknown-unknown`. These are product selections with defaults;
see candidate `packaging/{linux,macos,freebsd}/package.sh`,
`packaging/windows/package.ps1`, and `packaging/web/package.sh` plus
`apps/solidcraft-web/{Trunk.toml,index.html}`. A standalone Linux CLI
selection and a `--workspace` comparison were also resolved.

For each package specification (`rsa@0.9.10`, `crypto-bigint@0.5.5`), run:

```sh
cargo tree --offline --locked --target "$target" $selection \
  -e normal,build -i "$spec" --prefix depth --format '{p}|{f}' --color never
```

Repeat with `-e normal,build,features` for feature provenance. Development
edges are excluded. The broad workspace result includes roots not shipped
on the selected target; it is not a product link manifest.

## Exception-specific consumers and features

| Exception | Resolved features on all nine checked product targets | Complete reverse consumer set below the application crates |
| --- | --- | --- |
| `RUSTSEC-2023-0071` — `rsa 0.9.10` | `default,pem,std,u64_digit` | Sole immediate consumer: `pgp 0.21.1`. No other intermediary between `rsa` and the application crates. |
| `RUSTSEC-2026-0387` — `crypto-bigint 0.5.5` | `generic-array,rand_core,zeroize` | Immediate consumers: `cx448 0.1.1`, `elliptic-curve 0.13.8`. Remaining intermediaries: `ecdsa 0.16.9`, `k256 0.13.4`, `p256 0.13.2`, `p384 0.13.1`, `p521 0.13.3`, `primeorder 0.13.6`, `pgp 0.21.1`. No consumer path into the application bypassed `pgp`. |

For both crates, `pgp` resolves with `getrandom,wasm,web-time` and is
consumed directly by `solidcraft-io`. Beyond `solidcraft-io`, the complete
application consumer sets in the selected graphs are:

| Selection | Remaining application consumers for either exception |
| --- | --- |
| Native installer pair, eight targets | `solidcraft-engine`, `solidcraft-agent-task-engine`, `solidcraft-mcp`, `solidcraft-ui-egui`, `solidcraft-cli`, `solidcraft` |
| Standalone Linux CLI | `solidcraft-engine`, `solidcraft-mcp`, `solidcraft-cli` |
| wasm browser product | `solidcraft-engine`, `solidcraft-ui-egui`, `solidcraft-web` |
| Workspace comparison | Union of the native and browser sets; no additional intermediary consumer of either affected crate. |

## Owner risk decisions and verification

The owner accepted two separate temporary exceptions for this migration candidate. Neither acceptance is remediation or a blanket dependency-policy exception. No calendar expiry was specified. Recheck each on the final integration SHA, and reopen the decision if the audited boundary changes or a compatible patched route becomes available.

- **RSA:** the resolved graph supports a single entry through rPGP; it does
  not show which functions run. The existing source audit supports public
  verification rather than Rust private-key operations. The
  [RSA exception was accepted](https://linear.app/intgral/issue/INT-1454#comment-5a41c162-ffc1-4e6e-8977-fa163970dcd9); recheck the
  final SHA and the native GnuPG save/read boundary.
- **Crypto-bigint:** the broader graph includes `cx448` with `signing` and
  `elliptic-curve` with `ecdh` among enabled features. This is capability
  evidence, not evidence that production performs private signing or key
  agreement there. The [crypto-bigint exception was accepted separately](https://linear.app/intgral/issue/INT-1454#comment-5a1493fa-86f3-4eef-857f-f60a27c1f9c2);
  reassess any consumer or feature changes and compare a patched graph.

Raw reverse graphs, feature provenance and summary are retained at
`/tmp/opencode/int1454-b6-graph.8e3pHV/`. `graphs.sha256` covers 67 text
receipts, including standalone CLI normal/build graphs; its SHA-256 is
`3e79a5f915112a52192a84d404f1eb2a886834b537af1fb0db0dfefeceb48002`.
The eight additional Windows x86 graphs are recorded separately in
`windows-x86.sha256` (SHA-256
`20cb2dfee878dc24b7bf7701741e0d841af7ee645874fdeaaadc2c96bfada65c`).
Each matched the corresponding Windows x64 receipt byte-for-byte.
This is neither runtime validation nor approval to modify `deny.toml`.
