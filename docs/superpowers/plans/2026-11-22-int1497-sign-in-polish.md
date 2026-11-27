# INT-1497 Sign-in Polish Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make desktop device sign-in match Intcad's UI, copy the user code in one action, and hide unnecessary waiting-state details.

**Architecture:** Keep `sign_in_controls` and `waiting_sign_in_controls` in `crates/ui-egui/src/account/panel.rs` shared across account surfaces. Use UI Kit controls and per-prompt view state for disclosure and copy feedback; leave the host authentication state machine unchanged.

**Tech Stack:** Rust, egui, Intcad UI Kit, pending locale catalogs, desktop E2E harness.

**Spec:** The owner-approved login polish in [INT-1497](https://linear.app/intgral/issue/INT-1497): unified styling, short-code clipboard with feedback, concise waiting state, manual fallback, and shared behavior across Start, account menu, Assistant and Review.

## Global Constraints

- No contract, WorkOS settings, server deployment, or login method changes.
- Only the short user code may be copied, on an explicit action and before expiry. Never include the complete device URL, credentials, or stale codes in clipboard output.
- Browser launch still requires a trusted platform gesture. The manual fallback shows only a verified HTTPS origin and path, not query or fragment.
- Use shared spacing, buttons, disclosure and status text. Do not add a modal or change application-wide button rendering.
- Keep user-visible text localized. Use only synthetic codes in tests and screenshots.

## Review Focus

- Expiry while waiting hides copy and browser actions and preserves retry.
- A repeated or replaced attempt clears its predecessor's copy feedback and manual disclosure.
- Browser launch remains protected from synthetic or mixed gestures.
- Both narrow and normal widths keep code and actions visible; the Assistant fallback stays functional.
- Duplicate waiting text, the complete URL, and the countdown do not appear by default.

### Task 1: Shared sign-in presentation and interaction

**Files:** Modify `crates/ui-egui/src/account/panel.rs` and `crates/ui-egui/src/account/tests.rs`. Add `assets/locales/pending/account-sign-in-polish.json` for new strings.

**Interfaces:** Consumes `AccountView`, `DeviceCode`, and existing `Action` variants; keeps the existing host action interface. Produces per-surface device code copy and manual disclosure, with no authentication or network side effects.

- [ ] Add separate interactive Start and title-bar Sign in coverage in the account UI harness. Assert the default compact view, manual disclosure without query data, exact `OutputCommand::CopyText` content, stale/expired prompt, and feedback reset after a new challenge. Reuse the existing platform-gesture tests for browser activation and the actual no-WebView test.
- [ ] Run `cargo test --offline --locked -p solidcraft-ui-egui --lib account::` and verify the new copy and disclosure assertions fail for the expected missing behavior.
- [ ] Render one waiting status, a short code and Copy action, primary browser action, and secondary Cancel. Put the manual verification address behind a secondary disclosure. On valid explicit Copy emit only the short code and label its feedback “Copied”; key transient state by the surface and current challenge.
- [ ] Rerun account UI tests and `cargo xtask ui-containers`; run scoped formatting and catalog checks. Check normal and narrow desktop layouts with a synthetic challenge and capture screenshots without account data. Report missing platform or visual evidence explicitly.
- [ ] Commit the scoped UI change and record test and screenshot evidence in INT-1497. Do not push or deploy as part of this task.
