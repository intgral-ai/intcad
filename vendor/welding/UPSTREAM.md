# welding 0.15.0 — modified source

- Published package: https://crates.io/api/v1/crates/welding/0.15.0/download
- Package SHA-256: `e94298e32686b5d447c2abade5bb4594d4e2c4467a7116d3eb253fc7d230fed7`
- Repository: https://github.com/merely-made/wgpu-weld
- Published `.cargo_vcs_info.json`: revision
  `1836887569367fe8476b014ed8563d302a5c829e`, `path_in_vcs = "welding"`.
- Copyright: 2026 Mark Alan Boykin. Existing MPL-2.0 file headers retained.
- Copied only the published `src/`, normalized `Cargo.toml`, `build.rs` and
  `README.md`. No registry markers, nested lockfile, examples, assets or caches.
- The published package has no license text. `LICENSE.txt` is an unmodified copy
  fetched 2026-11-18 from Mozilla's canonical
  https://www.mozilla.org/media/MPL/2.0/index.txt ;
  SHA-256 `3f3d9e0024b1921b067d6f7f88deb4a60cbe7a78e76c64e3f1d7fc3b779b9d04`.

## SolidCraft local modifications

- `src/lib.rs`: export the owned runtime-identity module.
- `src/frame_geometry.rs` (new), `src/lib.rs`: export owned capture geometry,
  element, generation and mailbox-watermark scalars, with checked std-only
  full-surface validation and hostile-input fixtures.
- `src/runtime_identity.rs` (new): std-only checked identity values and validation
  tests; Linux+cef-runtime native queries and null-export test. Manual FFI and
  C-string copying stay here. No runtime owner is made by inspection.
- `src/runtime.rs`: `CefRuntime::inspect_identity` uses the existing loading path
  and native queries before initialization. On Linux the existing loader contract
  uses the linked `libcef.so` selected by `LD_LIBRARY_PATH`, not a new dlopen owner.
  An opt-in `command_line_args_disabled` config field maps to the real CEF
  Settings field before initialization. It defaults to false; the app enables
  it only for Wayland OSR so inherited browser argv cannot bypass approved
  callback switches. Subprocess execute-process arguments are unchanged.
- `src/surface.rs`: `NavigationEvent::BrowserCreated` carries browser id and OSR mode.
- `src/linux_cef/cef_backed.rs`: emit that event, close hosts violating OSR mode,
  and add callback tests separating denied browser popups from PET_POPUP widgets.
- `src/linux_cef/{mod.rs,cef_backed.rs}`: copy the actual common CEF capture
  scalars and validity flags into owned Linux images; reject nonpositive signed
  coded sizes before fd duplication; return mailbox lock errors (callbacks log
  and refuse); expose view/popup discard watermarks for host admission reset.
- `src/linux_cef/mod.rs`: expose explicit `set_page_focus(bool)` through the
  existing browser-host API so hidden OSR pages can be blurred. Backward focus
  traversal remains unchanged and is not used as blur. Linux resize/scale setters
  return recoverable errors for absent hosts or poisoned metrics locks instead
  of acknowledging a failed transition or panicking.
- `src/native_frame/{mod.rs,vulkan_dmabuf.rs}`: carry optional capture metadata
  with the same owned DMABUF and imported texture. Constructors default to None.
  Discard preserves generations; counters saturate, and Linux refuses exhausted
  identities through checked generation allocation. Add real fd/mailbox
  regressions (native execution must be validated separately).
- `src/native_frame/{dx12.rs,metal.rs}`: initialize imported capture metadata
  to None without changing the platform import paths.

No changes to native CEF/Chromium, GPU import mechanics, fd duplication/ownership,
synchronization, OSR select/fullscreen behavior or platform context-menu routing.
App-side policy is outside this package. Local changes are marked `SolidCraft:`.

Remove the patch when a published welding release provides equivalent checked
pre-init identity, browser-mode reporting/enforcement and owned native capture
metadata transport, after revalidating the native Wayland OSR scope and all
supported platform builds.

## Distribution obligations

This is **modified MPL-2.0 source**, not an unmodified registry dependency.
Binary distributors must make the exact modified covered files available under
MPL-2.0, retain headers and license, and tell recipients where to obtain that
source. Publish this tree with the matching Intcad release/source revision.
Linking only the original crate is insufficient. See
`packaging/notices/WEB-VIEW-PLATFORM-NOTICES.txt`. This record is not legal clearance.

## Original published file SHA-256 (before local modifications)

```text
50a8e3bd7b77bdad4f56d0aaaa6049b6610d83e6cd5cba1be1b6a96bf08cfc4a  Cargo.toml
4c87f244776e98c9211307a7f70959d5123c207b4b22d7c3bf56dbfcb2a79e88  build.rs
5627e853055c4f864dcad2f991d70d25d10464207764dff2fb0a37dd7583498e  README.md
04be54e37799c7ec6e8b4d14ddb9c4a5432ac7ca49df975a9d89d5fcd7c495a7  src/app.rs
7d1690c65ddd91a1d5300057fae36b4539c4aeaea8b11d8d2f6ac4a88920a48f  src/auth.rs
45c26161e10d43278d2b8ea8a57ec4e3edf65be25eb6c9c730eac161ac074d63  src/cef_input.rs
cebf4b91ea3a42389bf7cbeef8e7d9a6bf0c4641751ec380306c8446184d0d64  src/cookies.rs
6672a1c8e0521a5733b362fd0b0500d288a81644de7251c253ee16505afdc88d  src/cursor.rs
ef5f908a028744745e6712008d5e0da2ba6166f02be46d2d4f59a497a7d8f7c2  src/devtools.rs
048c125ddd7f642d0a061bb6b4e363c969753a0c78f980f39496bd772330abe7  src/downloads.rs
6af4a01b104c32830091b3aca9beeace66c36ed602a4765fb9805dcbc5ed1a21  src/drag.rs
98a6aa78701a3ea1860041716993021c975558a762f35d0c08d816d480c16860  src/error.rs
2bc993815b66ab0e630a9124c6a37ca7d8b2399f63bee8cef624fc8678304745  src/ime.rs
7573ba02f045b3eeeb2d21ec8ee6b7017df5c22bb7c6e074d3060cbf5dc20f50  src/lib.rs
0ae678df7fc11acc8488aa2118c435983eb05186bb058e107c01965688572431  src/linux_cef/cef_backed.rs
1ce87a913b5f2a9ac4c717bb054163b42d83866755825c4f9d9f001a533576d0  src/linux_cef/mod.rs
a77e0ffd6928f313d94ad93911a04b6d4ec80f851abdae2e320c7a2c2a3967e0  src/macos_cef/cef_backed.rs
23082ca70856ea8b4b32a6881a15af5c60c7ec05f8b485a2387a20667a2fb70e  src/macos_cef/mod.rs
fa8b1eab03a31762779bf6d5306822854511e9355b59d16e433260ce0ad883d5  src/native_frame/dx12.rs
63ce0a02ba0433f6757b8fe8729f14bf2bf9e55f647f8a9fc0ece0da7a5249f2  src/native_frame/metal.rs
6eacc37d69d95659e0c9edb7448c83c41c9549df1b43025b8f0b08d09fb6e9bf  src/native_frame/mod.rs
3b624165524aedf2e5cf1ec7f5b94cf9091fdd38167d2cc5089867d2f15a8e3b  src/native_frame/vulkan_dmabuf.rs
17a133dcfa9cb01001331a6b422be600b66c4d13fa4d1b4d577ad3e038e690c0  src/permissions.rs
954e1011091a3b9646361fb2b8244c78cb6de5016ade5766fcb23853892cd5f2  src/popup.rs
f1586e492d763c2ae1c506d08839908e623b9919c1d69249c6c48c8f72280dad  src/profile.rs
1abbd5d6dd13efc60918b5be6423d8306b226b95fba0fa91111d708a870f53df  src/runtime.rs
c2c164e62596be7afadc6608dbfc52f1416a2f2f5c87e38c22d0069157661892  src/snapshot.rs
45cdfe7c2b8f03dc6849e47447dc6fc7f801a991b3e951bf6fe7002abd7c27a9  src/surface.rs
6ccaf96207a09be5e82a03ebb59a2cd04985cd2f429d063554cc2574a0e33302  src/view.rs
b003ef191620981ac79b55bd70f486bc8ac585025194cae8db2c422d2547fb1a  src/wgpu_compat.rs
ef9d3c28a3683cb7ba7966129ccf512c9f3776940c08dab3a1598c65b8ec804b  src/windows_cef/cef_backed.rs
c9a04aed824329c4f74c65e88071467eb33b46fc086d7eb6aef537c600beb045  src/windows_cef/mod.rs
```
