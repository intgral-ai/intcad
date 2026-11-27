# Model and files

## Sub-features

Cancel and confirm a feature, undo and redo it, switch native panels, save a design and reopen it.

## How to get to it (user POV)

In a new design, sketch a rectangle on XY, extrude it, and use Inspect → Measure.
Use File → Save As, close the design, then open the saved file.

## Driving it with the desktop control channel

From a clean task worktree with a desktop display and Python 3.9+, use the pinned Rust
toolchain and a worktree-local Cargo target. Set `INTCAD_VERIFY_EVIDENCE` to a new absolute
directory outside the checkout.

```sh
cargo build --locked -p solidcraft --bin intcad
python3 .agents/skills/verify-intcad/scripts/desktop-smoke.py \
  --binary target/debug/intcad --evidence "$INTCAD_VERIFY_EVIDENCE"
```

On Windows use `target/debug/intcad.exe`. If the Cargo target is elsewhere, pass its binary.
The script checks the running commit and creates a rectangular profile. It opens the native
Extrude form and clicks Cancel, which must leave zero bodies. It reopens the form, types
20 mm and clicks OK. Toolbar Undo must remove the solid and Redo must restore one closed
40 × 30 × 20 mm body of 24,000 mm³. It then opens Review, checks for its Run control
and switches to Files. The model must retain the same dimensions and volume. Finally it
saves without an evaluation cache, closes the design, reopens it and measures again.
Keep the JSONL transcript, saved design, model capture and app log from the evidence directory.

## Gotchas

For toolbar, shortcut or native picker changes, drive the actual control and capture the window.
The script's `ui.render` capture shows the model even if the window is covered. For an import
change, open a synthetic fixture in the affected format and check the geometry and a
format-specific property.
