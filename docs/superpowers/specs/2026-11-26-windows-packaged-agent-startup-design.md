# INT-1481: Windows packaged Assistant startup

The exact Windows x64 v0.1.3 staging MSI installs and runs CAD, but the packaged Assistant cannot initialize on a clean Windows 11 guest. Node 22.23.4 fails before loading the entry when the desktop passes Windows verbatim paths. A standard absolute path gets past that failure and exposes a second one: the packaged libSQL addon directly imports `VCRUNTIME140.dll`, absent on the guest.

## Approach

- Keep the current packaged Node, pin, ACP stdio transport and desktop-owned account authority. Keep canonical path and package identity validation; pass Node a compatible absolute path only when it still names the validated file. Refuse a path that cannot be represented safely. Never fall back to another Node, entry or shell.
- The owner selected shipping the required Visual C++ runtime with the Windows installer instead of asking users to install it separately. Prefer application-local deployment next to the packaged Node, and include it in the Windows x64 portable ZIP too because it ships the same Assistant. The product decision is not a redistribution approval. Ship only Microsoft-signed, x64 runtime files needed by the installed child, from a reviewed Microsoft redistribution source and list, with exact provenance, hashes, and required terms. If application-local loading or redistribution cannot be validated, stop and bring back the specific finding rather than silently adding a global installer.
- No change to x86/arm64 packages which do not carry the Assistant, other platforms' runtime layouts, or the single Windows `.exe` that does not carry the packaged Agent. No first-run download, system-wide installation, new npm/Cargo pin, auth relaxation, or new known-failure exemption.

## Acceptance

Fail the tests on the known candidate, then exercise the production launch path through the installed desktop, not only a direct Node probe. On clean Windows without preinstalled Visual C++ runtime, demonstrate ACP readiness, launch → stop → relaunch, and load the shipped native dependency from the intended package location. Verify both x64 package layouts and leave existing offline CI smoke intact. With a fresh staging build at the new protected source, complete Linux then Windows authenticated acceptance, including manual **Allow once**, native CAD readback, one admission and one successful settlement, cancellation, offline and sign-out refusal. Reuse no previous run as candidate approval. Preserve the protected release gate and separate human approvals for signing and publication.
