# Model and files

## Sub-features

Build and measure a solid, save a native `.sc` design, close it and open it again.

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
The script checks the running app's commit, then uses the public modeling commands to make a
40 × 30 × 20 mm solid. It expects one closed body of 24,000 mm³. It saves without an evaluation
cache, closes the design, opens the file through the desktop's Open path and measures again.
Keep the JSONL transcript, saved design, final screenshot and app log from the evidence directory.

## Gotchas

For toolbar, shortcut or native picker changes, also drive that actual control. A desktop
screenshot needs a presented frame. For an import change, open a synthetic fixture in the
affected format and check the imported geometry and a format-specific property.
