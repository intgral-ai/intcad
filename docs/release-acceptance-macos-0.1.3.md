# IntCAD 0.1.3: macOS acceptance

**Status: NOT TESTED on the merged candidate.** Use the exact DMG from the reviewed
`develop`-based candidate containing the Assistant fix. Record its source SHA, workflow
run and attempt, DMG SHA-256, hardware and macOS version. Do not substitute a source
build or the earlier rehearsal DMG.

The nonpublishing [workflow](../.github/workflows/release.yml) builds both slices and
calls [`package.sh`](../packaging/macos/package.sh) without `--sign`: ad-hoc app and CLI
signatures, unsigned DMG, no notarization. [`installed-check.sh`](../packaging/macos/installed-check.sh)
copies out the app, verifies its local signature and opens the Assistant offline. Its
Gatekeeper rejection is nonfatal, and its x86_64 test can be skipped or run under Rosetta.
None of that proves a first Finder launch, native Intel execution, or visual acceptance.

## Inspect the delivered bytes

Set `DMG` to the downloaded file and `APP` to `Intcad.app` on a read-only mount.
Compare the digest with the recorded candidate. Record each command's verdict separately.

```sh
shasum -a 256 "$DMG"
hdiutil verify "$DMG"
for bin in "$APP/Contents/MacOS/Intcad" "$APP/Contents/MacOS/intcad-cli"; do
  lipo -archs "$bin"
  lipo "$bin" -verify_arch arm64
  lipo "$bin" -verify_arch x86_64
done
plutil -p "$APP/Contents/Info.plist"
codesign --verify --deep --verbose=2 "$APP"
codesign --display --verbose=4 "$APP"
codesign --verify --verbose=2 "$DMG"
xcrun stapler validate "$DMG"
xcrun stapler validate "$APP"
```

Expect `ai.intgral.intcad`, version 0.1.3, and both slices in both programs. An absent
signature or staple is evidence to record, not permission to bypass policy. Use strict
signature verification on the copied app rather than the HFS mount. A local signature
pass does not establish Developer ID trust or notarization.

## First launch and visual matrix

1. Use a fresh profile on physical Apple silicon and Intel Macs. Preserve quarantine:
   check `xattr "$DMG"` before opening, drag the app to Applications in Finder, eject,
   then check `xattr "$APP"` on the copy and double-click normally. Record any rejection
   verbatim; mark launch and visuals **NOT TESTED — blocked** if policy denies launch.
   Do not strip quarantine, re-sign, add an exception, or substitute direct execution.
   Run `spctl --assess --type execute --verbose=4 "$APP"` after first-launch observation
   or on a separate inspection Mac. An absent quarantine attribute is not a quarantined
   first-launch test.
2. On each available Mac use Light and Dark at 100% and 150% app scale. Add a 125%
   replay for the reported fractional case. Record macOS display scaling separately.
   Compare built-in and external Agent composers at the same width and scale with an
   unsent draft and a credential-free local fixture. Keep CAD zoom fixed.
3. On Sample Plate, expand the Assistant and collapse it with a viewport click. Capture
   the transition and the settled state without further input. Separately repeat with
   pointer movement and with wide-to-narrow resize. Use ordinary and error status.
   Require no exposed backing, stale pixels or clipped glyphs. Reopen and check draft
   and caret. Dismiss a menu, then verify the composer accepts typing and the first
   click outside it reaches the model.
4. Record About/version and clean quit. Keep per-cell screenshots or recordings with
   candidate digest, configuration and verdict. Mark missing hardware, cases or fixtures
   **NOT TESTED**. Do not remove the bundled CLI to simulate the old missing-CLI warning
   on candidate bytes. Gate B, slash commands and one-click update retain separate verdicts.
