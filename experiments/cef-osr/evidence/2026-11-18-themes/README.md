# 2026-11-18 theme matrix

25 built-in themes, applied one after another in a single running IntCAD through the app's own
control channel (`ui.prefs {theme}`), with the app's screenshot saved for each.

| File | What it is |
|---|---|
| `<theme>.png` | the Assistant panel of that theme, cropped from the app's screenshot and half-scaled (full-size images were produced by the run) |
| `matrix-1_25.json`, `matrix-1_0.json`, `matrix-2_0.json` | per-theme: applied, reported id, dark, expected surface token, the surface actually painted, delta, screenshot path |
| `dpi200-nord.png`, `dpi200-1-bit.png` | the same panel at 200 % DPI, dark and light |

Results: 25/25 themes matched their surface token at 100 %, 125 % and 200 % DPI (tolerance 2/255
for egui's dithering; the only difference seen was one step on `solidcraft-95` before the
tolerance was recorded).

Reproduce:

```sh
CEF_PATH=... scripts/theme-matrix.py <path-to-intcad> --out /tmp/themes --dpi 1.25
```
