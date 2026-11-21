# Account architecture and trust boundaries

The account client keeps the pure entitlement rules in the engine and the native-only adapters in
a crate that no browser target compiles. The [entry matrix](entry-matrix.md) classifies each
host entry; [baseline](baseline.md) records the frozen contract. This note describes the seams, not
production enablement.

| Crate | Role | Browser safety |
|---|---|---|
| `solidcraft-engine::entitlement` | admission, budget, commit-gate and operation-store interfaces; hosts supply identity, time and billing through ports | wasm safe: no network, OS credential or wall-clock calls |
| `solidcraft-ui-egui` | account and credit UI; the composition root installs ports, and none is installed by default | wasm safe: no account adapter dependency |
| `solidcraft-account-native` | durable operation ledger, OS credential store, HTTPS transport and server-verified product session | native only (`#![cfg(not(target_arch = "wasm32"))]`); never in a browser closure |
| `apps/solidcraft`, `apps/solidcraft-cli` | native composition roots; the only crates that currently name `solidcraft-account-native` | not browser targets |
| `apps/solidcraft-web` | browser runner and its existing `window.solidcraft` control API | free individual entry, anonymous, no account dependency |

## Seams

- The engine owns pure ports and decisions. A controlled operation is admitted only from a
  server-issued grant; local affordability never mints one.
- The desktop composition root creates the account session, operation store and credential store.
  The UI receives injected ports and has no keyring or HTTP implementation of its own.
- `solidcraft-account-native` owns the native transport boundary. It does not decide product
  entries, and it does not run on the window thread.
- The Web API keeps its current free entry category and never constructs or links a native
  account adapter.

## Trust boundary

The server verifies the provider session and returns the account, protocol and client fields the
client consumes. The client does not trust token shape alone, does not follow redirects, and does
not activate a product session from a provider login alone. Product credentials stay in the
native OS-protected store. Passwords, keyring data and service tokens never enter the browser
bundle or Web storage.

At this checkpoint no production sign-in, service origin, settlement authority or UI wiring is
enabled; those remain gated by the surrounding tasks in the plan.

## Closure guard

`cargo xtask layers` resolves the locked package graph and rejects `solidcraft-account-native`
below `solidcraft-web` or `solidcraft-ui-egui` on every target. It also rejects
`solidcraft-account-native`, `keyring` and `ureq` in the `wasm32-unknown-unknown` closure of every
layer crate and the web app. `cargo xtask wasm` runs the same graph guard before compiling. The
guard evaluates cargo target conditions for wasm and keeps unknown conditions in the closure, so
it can fail closed but cannot silently narrow.

This is dependency-graph verification, separate from a wasm compilation result. To inspect the
locked closures directly:

```bash
cargo tree --offline --locked --target wasm32-unknown-unknown --edges normal,build -p solidcraft-web
cargo tree --offline --locked --target wasm32-unknown-unknown --edges normal,build -p solidcraft-ui-egui
```
