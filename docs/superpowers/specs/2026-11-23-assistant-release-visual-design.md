# Assistant visual release gate for IntCAD 0.1.3

Keep the bottom-centre Assistant from #221. At the same application zoom and viewport width, switching between an external agent and the built-in agent must not change the composer's typography or basic control spacing. Use the built-in composer's current sizing as the reference; do not change overall desktop or CAD zoom, non-centred chat, or camera behaviour.

When the user collapses the conversation, retain the existing compact status row. It must be readable or completely hidden when there is no room. The collapsed surface must have no clipped glyphs, stale conversation pixels, or exposed backing rectangle outside its intended shape. Opening, closing and resizing must still preserve focus and let users interact with the model outside the Assistant.

## Approaches

- **Targeted repair:** align the shared browser sizing for the centred composer, then fix the height and backing contract at the native boundary identified by the Linux reproduction. This retains the required layout and minimises changes to Windows and macOS.
- **Global zoom or sidebar rollback:** global zoom changes unrelated controls without fixing native clipping. Sidebar rollback drops required scope. Neither is the release fix.

## Evidence and release constraints

Reproduce in the Linux desktop app, not just in a browser. A red-before/green-after regression must exercise the offending path. Inspect Linux and Windows at 100% and 150%, light and dark, including collapsed error status, reopening, narrow-window resize, and a model click outside the composer. Verify macOS separately on a real Mac; mark it NOT TESTED if unavailable. Use source previews for diagnosis, not release acceptance. Accept installed bytes from the same merged-develop-based candidate SHA, separately track login, slash commands and one-click update, and do not publish without explicit CTO approval.
