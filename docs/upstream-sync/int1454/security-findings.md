# INT-1454 Phase 0 — security findings carried by the sync

These were found while mapping the assistant/MCP/control units. Each was re-checked by the
integrator against `origin/develop` (`0fd92bea2`) and `v0.44.1` (`e151d5ea6`); the commands are
in the last section. Findings here are about **today's IntCAD**, not about the sync.

## SEC-1 — agent-chat gate is bypassed by command-id spelling (live, high)

Upstream fixed this in `7b22bb3e2` "Harden command-id matching for agent chat and plugins".

**Structure (verified):**
- `crates/ui-egui/src/agent_chat/gate.rs:117` trims the id, then matches with **case-sensitive**
  `starts_with("diagnostics.")`, `starts_with("app.")`, `starts_with("prefs.")`,
  `starts_with("file.") || starts_with("doc.")` (`:118-126`).
- `crates/engine/src/cmd/mod.rs` `find_command` falls back to `eq_ignore_ascii_case`.
- For `engine.execute`, the gate reads **only** `params.command` (`gate.rs:157`), while
  `crates/ui-egui/src/control.rs` `execute_command` also accepts a string `id`.
- `METHODS` admits `engine.execute`, so the request is accepted and the gate decides on text that
  the engine will not use verbatim.

**Consequence:** `{"command": "FILE.SAVE", …}` (or `DOC.OPEN`, `APP.QUIT`, `PREFS.…`,
`DIAGNOSTICS.CRASH_REPORTS`) reaches `Verdict::Run` instead of `Ask`/`Refuse`, and the engine then
resolves the id case-insensitively and runs it. The same holds for a request that carries `id`
instead of `command`. A prompt-injected agent can therefore write, open or quit without the
approval card.

**Why tests miss it:** the existing gate test only feeds registered spellings
(`every_registered_file_and_doc_command_asks`), so it never exercises case or alias forms.

**Upstream shape of the fix:** `resolve()` canonicalises to the registered id, `command_of()` reads
`command` or `id` as the control channel does, unknown ids become `Verdict::Unknown`, and
`plugin.run` always asks. IntCAD's own additions to the gate (`file.open_in_easyprint`, slicer
rules) must be preserved.

**Disposition:** adopt now, as a small standalone PR, not as part of a large unit. It restores the
approved gate; it does not change the security model. It does change behaviour for automations that
send non-canonical ids — they will be asked instead of silently running.

## SEC-2 — STEP import string decode is quadratic on hostile input (live, medium)

Upstream fixed this in `54b4bb3f4`; the fixed decoder is in the vendored
`solidcore/crates/solidcore-record/src/step/p21.rs` at v0.44.1 (`decode_string_with`, documented
as linear, with a 100,000-tag test). The same fix is **not** in IntCAD's `crates/step/src/p21.rs`,
which still does `rest.windows(4).position(|w| w == b"\\X0\\")` inside the loop for every
unterminated `\X2\`/`\X4\` run — O(N²) over the file.

Reachable by opening a hostile `.step` inside the allowed paths, or through the UI importer.

**Disposition:** either port the 49-line fix into `crates/step` now, or let the STEP/solidcore
migration deliver it and record the interim risk. The migration is the only place that removes
`crates/step`, so the fix must not be lost in the move — the upstream snapshot already carries it.

## SEC-3 — connection-limit refusal can still reach the client as a reset (low, availability)

Correction after review: IntCAD **does** send `{"ok": false, "error": "too many connections"}`
(`apps/solidcraft/src/control_server.rs`, in the `OPEN.fetch_add(...) >= MAX_CONNECTIONS` branch).
What upstream `18d82f584` adds is the part after the write: drain the unread request before
closing, share the open counter and max-line settings, and name the constant
`TOO_MANY_CONNECTIONS`. Without the drain, a client that is still writing can still see a TCP
reset instead of the refusal, which is what made Windows clients report a crash. No access gain;
a small, contained refinement rather than a missing fix.

## SEC-4 — defaults that must not be adopted silently

| Upstream commit | Change | IntCAD today | Rule |
| --- | --- | --- | --- |
| `12a9a7f0f` | "Let AI agents connect" defaults **off**, live toggle | defaults **on** (`ui-egui/src/prefs.rs`) | changed default → preference + human sign-off |
| `4d2691e63` | opt-in `allow_insecure_http` lets a key go to plaintext private endpoints | stricter: key only for `https://` or loopback | do not adopt without the consent surface; adopting weakens a boundary |

## SEC-5 — plugin runtime (not applicable today)

Upstream's plugin host (`53a76b23f`, `2167af4d6`) is Wasmtime-sandboxed with per-folder grants,
hidden-folder symlink refusal and archive-slip protection. IntCAD has no plugin runtime
(`crates/ui-egui/src/plugins.rs` is a metadata list only), so there is nothing to harden yet.
If plugins are ever adopted, `safe_zip` and the link-component rule are mandatory, not optional.

## Evidence commands

```sh
git show origin/develop:crates/ui-egui/src/agent_chat/gate.rs | sed -n '110,200p'
git show origin/develop:crates/engine/src/cmd/mod.rs | sed -n '/pub fn find_command/,+4p'
git show origin/develop:crates/ui-egui/src/control.rs | sed -n '/fn execute_command/,+6p'
git show origin/develop:crates/step/src/p21.rs | sed -n '348,388p'
git show v0.44.1:solidcore/crates/solidcore-record/src/step/p21.rs | sed -n '655,700p'
git show --stat 7b22bb3e2 54b4bb3f4 18d82f584
```
