# INT-1497 Desktop Account Entry Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development or superpowers:executing-plans. Steps use checkboxes.

**Goal:** Connect the confirmed desktop account entry to WorkOS AuthKit without changing anonymous CAD or the server authorization contract.

**Architecture:** Extend the existing desktop account port and UI, rather than adding password forms, a deep-link callback, or a Railway login page. The native device-code worker polls outside the UI thread; the existing server-verified exchange verifies the product session before the native credential lifecycle publishes it.

**Tech Stack:** Rust, egui, existing `solidcraft-account-native` device-code transport and protected credential store.

**Spec:** [INT-1497, §3 首版登录、注册与密码找回入口](https://linear.app/intgral/issue/INT-1497), confirmed by the user. WorkOS hosts sign-in, registration, email verification and password recovery. On successful device confirmation, Intcad notices the session; users switch back manually and retry their own actions.

## Global Constraints

- Desktop only; CLI/MCP remain non-interactive and never launch a browser automatically. No website, password collection, new server endpoint, frozen contract edits, deploy or remote writes.
- Only an explicit “Sign in in browser” UI action may open the system browser. Accept only a documented trusted HTTPS verification origin; never launch an arbitrary URL or put tokens in logs.
- Provider completion is not product authentication: reuse `ServerVerifiedProductSession` and the approved status exchange. Missing trusted configuration, verification, or secure-store support cannot silently enable access.
- Worker completion must be scoped to the current login attempt. Cancel, sign-out, switch, shutdown and late replies must not install a stale session. Do not replay a blocked operation or infer execution authority from login.

## Review Focus

- Explicit click versus passive rendering, refresh, refusal and programmatic actions.
- Device expiry, retry, rate-limited polling and errors without blocking the window.
- Canceled/obsolete worker completion and product verification failure.
- Anonymous CAD, headless behavior, secure storage and untouched server contract.

### Task 1: Desktop login and product-session activation

**Files:** `apps/solidcraft/src/account.rs` and a focused desktop worker module if needed; `crates/account-native/src/device_code.rs` and existing tests if needed; `crates/ui-egui/src/account/{mod,panel,tests}.rs`, existing Chat/Review refusal surfaces, pending locale file, and `docs/account/qa-evidence.md`.

- [x] Write and run failing tests in existing account suites: explicit desktop click starts one attempt; no auto-launch; successful product verification updates the view without replay; failure/expiry permits explicit retry; canceled/obsolete attempts do not activate or persist a session.
- [x] Wire the existing `AccountPort`/`State::process_actions` lifecycle to a bounded native worker, reusing `DeviceCodeFlow`, `CredentialSession` and `ServerVerifiedProductSession`. Use explicit trusted configuration for the WorkOS public client ID and product HTTPS origin. Keep browser-launch and network seams injectable for tests. Do not introduce new account authority.
- [x] Present the signed-out desktop menu, local Chat/Review prompts, pending/expiry/retry affordances. Reuse UI kit and localization conventions; no effect on wasm or headless.
- [x] Run focused native, desktop and UI tests as supported, plus diff and relevant scope checks. Note actual failures and all unrun GUI/WorkOS/three-platform acceptance separately. Commit locally and report RED/GREEN.

Evidence: `docs/account/qa-evidence.md`; task report
`.superpowers/sdd/2026-11-21-int1497-desktop-entry/task-1-report.md`.
