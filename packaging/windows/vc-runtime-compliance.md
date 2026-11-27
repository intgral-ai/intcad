# Visual C++ runtime: Windows x64 Assistant

**Status: technical receipt recorded; PP confirmation of the payload and downstream terms is recorded separately below.** The files are staged only from the licensed source below and are verified on every build.

## Build and binary evidence

| Evidence | Observation |
| --- | --- |
| [Staging `45908354228:1`, Windows x64 job `140215823957`](https://github.com/intgral-ai/intcad/actions/runs/45908354228/job/140215823957) and [promotion gate `45895693088:1`, Windows x64 job `140175019694`](https://github.com/intgral-ai/intcad/actions/runs/45895693088/job/140175019694) | `windows-latest` resolved to `windows-2025-vs2026`, image `20261116.315.1`. The jobs used `rustc 1.99.1`; the packaged Agent used Node `22.23.4`. Neither log prints the complete MSVC executable versions or the app-local source directory. |
| [GitHub's manifest for that runner image](https://github.com/actions/runner-images/blob/win25-vs2026/20261116.315/images/windows/Windows2025-VS2026-Readme.md) | Visual Studio **Enterprise** 2026 `18.12.12405.243` (release [18.12.0](https://learn.microsoft.com/en-us/visualstudio/releases/2026/release-history)); installed x64 v14 runtime `14.52.37029`. That image also lists an older 14.44 toolset component. The manifest is not a statement of Intcad's license or the source of release DLLs. |
| Exact same-run Windows x64 `intcad.exe` | Its PE linker fields are `14.52`; its Microsoft Rich-header records include build `37029`, consistent with MSVC `14.52.37029`. Do not infer the exact `cl.exe` or `link.exe` file revision from these fields. Capture both executable versions and `VC\Tools\MSVC` directory identity on the next Windows job without installing, selecting or updating a toolset. |
| Four PE files under `mastra-agent` in the exact same-run portable ZIP | `@libsql/win32-x64-msvc/index.node` and `@ast-grep/napi-win32-x64-msvc` directly import `VCRUNTIME140.dll`; `node.exe` and `bufferutil` have no direct VC runtime import. The libSQL PE linker fields are `14.44`. Imports alone do not prove the complete transitive or dynamic set. |

The staging Windows installed-check step passed but expressly logged `NOT VERIFIED: the built-in agent starts, reports connections and starts again (needs account authority)`. Its overall success is not proof of that process lifecycle; preserve the existing smoke and obtain positive exact-package evidence.

The [official Community Stable launcher](https://aka.ms/vs/stable/vs_community.exe) was downloaded for metadata inspection only, not run. Its file version was `18.12.12412.261` (18.12.1 per release history), Microsoft Authenticode status `Valid`, and SHA-256 `efdfe9d22349a567228e27b88da27564e964f78e1749dd94a6e5a634c06bd252`. This is the bootstrapper hash, **not** a runtime DLL hash or confirmation of which CRT bytes its layout would supply.

## Microsoft terms to review

- [Visual Studio 2026 Community terms](https://visualstudio.microsoft.com/wp-content/uploads/2025/10/Visual_Studio_2026-License-Community_ENU.docx), §§2 and 8; [Visual Studio 2026 REDIST, Visual C++ Runtime Files](https://learn.microsoft.com/en-us/visualstudio/releases/2026/redistribution#visual-c-runtime-files). The matching stable Community edition is the proposed license channel. The [2022 Community terms](https://visualstudio.microsoft.com/wp-content/uploads/2021/11/Visual-Studio-2022-Community-License-EN.docx), §§1 and 4, and [2022 REDIST](https://learn.microsoft.com/en-us/visualstudio/releases/2022/redistribution) govern 2022 acquisitions, not the observed VS 2026 runner or a 2026 payload.
- An individual may use Community to develop their own application. For organizational proprietary work, verify the non-enterprise allowance and the maximum five concurrent Community users. Apply the enterprise test to the organization and its affiliates collectively: more than 250 PCs or users, or the EULA's US$1 million annual-revenue threshold. An OSI-licensed upstream or dependency does not establish the exception for a proprietary distributed application.
- The unmodified listed files must ship with a program that adds significant primary functionality. Preserve supplier notices, avoid Microsoft branding that implies endorsement, and observe the restrictions on excluded licenses and unreleased components. External end users and downstream distributors must agree to terms at least as protective of the files as Microsoft's terms. A notice file alone does not establish agreement. Review the 2026 indemnify/defend/hold-harmless clause and its exception.
- The [official deployment guide](https://learn.microsoft.com/en-us/cpp/windows/redistributing-visual-cpp-files) describes application-local deployment; Intcad, not Windows Update, must service the private copies. The [v14 compatibility rules](https://learn.microsoft.com/en-us/cpp/porting/binary-compat-2015-2017#restrictions-on-binary-compatibility) require a runtime at least as new as the applicable build tools. Microsoft [supports Community on the latest Stable release only](https://learn.microsoft.com/en-us/visualstudio/releases/2026/release-history); its published fixed-version installers do not list Community. Do not presume that the current Stable installer supplies the image's exact `14.52.37029` bytes.


## Payload receipt (machine-checked)

`packaging/windows/vc-runtime-files.json` is the authoritative receipt. Current contents:

| Field | Value |
| --- | --- |
| Source | Visual Studio Community 2026 Stable, official Microsoft installer `https://aka.ms/vs/stable/vs_community.exe`, version `18.12.12412.261`, SHA-256 `efdfe9d22349a567228e27b88da27564e964f78e1749dd94a6e5a634c06bd252`, valid Microsoft Authenticode |
| Installed component / path | `Microsoft.VisualStudio.Component.VC.Tools.x86.x64`; `VC\Redist\MSVC\14.52.37025\x64\Microsoft.VC145.CRT\vcruntime140.dll` |
| File | `VCRUNTIME140.dll`, AMD64, file/product version `14.52.37029.0`, 176,488 bytes |
| SHA-256 | `eb1ee979bf02324712346e5bdbfc59c785388bc50d18cc2688aae4b0734915a0` |
| Signature | `CN=Microsoft Windows Software Compatibility Publisher, O=Microsoft Corporation, …`, status Valid (verified by the staging helper) |
| REDIST basis | [Visual Studio 2026 REDIST, Visual C++ Runtime Files](https://learn.microsoft.com/en-us/visualstudio/releases/2026/redistribution#visual-c-runtime-files): unmodified files within the licensed `VC\Redist` tree, shipped as part of the installation package |
| Target | `mastra-agent\node_modules\@libsql\win32-x64-msvc\VCRUNTIME140.dll` and `mastra-agent\node_modules\@ast-grep\napi-win32-x64-msvc\VCRUNTIME140.dll` |
| Loaded by | the packaged Agent's `@libsql/win32-x64-msvc` and `@ast-grep/napi-win32-x64-msvc` native modules |
| Compatibility | 14.52 is at least as new as every component's build tools (libSQL linker 14.44; the Rust executable 14.52); the v14 runtime is binary-compatible back to 2015 |
| Not included | `debug_nonredist`, `onecore\debug_nonredist`, previews, merge modules, the rest of `VC\Redist`, `System32` copies and machine-wide installers |

Verification on the Windows 11 verification VM (the exact installed staging Agent's folder, with
the runtime staged by `vc-runtime.ps1` from the Community install):

- `vc-runtime.ps1 -Action Stage` and `-Action Verify` both passed against this receipt.
- The installed Node `22.23.4` answered ACP `initialize` from `agent/main.mjs` (Mastra Code
  1.19.1) with the staged folder as its only source.
- `@libsql/win32-x64-msvc` and `@ast-grep/napi-win32-x64-msvc` both loaded and ran; PowerShell
  read the running Node's loaded modules and found `VCRUNTIME140.dll` at
  `…\mastra-agent\node_modules\@libsql\win32-x64-msvc\VCRUNTIME140.dll`.

Clean-machine proof (Windows 11 Enterprise Evaluation, no Visual Studio, no
`C:\Windows\System32\vcruntime140.dll`, no Intcad install): the portable tree was extracted
under `C:\Users\intcad\Desktop\Intcad Portable Test\` (spaces in the path), the receipt file was
placed beside each native module, and the installed Node answered ACP `initialize` (Mastra Code
1.19.1). `libSQL` and `ast-grep` both ran, and the running Node's modules listed
`…\@libsql\win32-x64-msvc\VCRUNTIME140.dll` — the package copy, not a system one. Repeating with
the file only beside `node.exe` failed on that clean machine. This is the portable channel's
loader proof; the exact new ZIP and MSI are verified again at the installed gate.

The initial "beside `node.exe`" placement was **not** used: Node loads a native module's
dependency from that module's own folder, and an empirical probe with the file only beside
`node.exe` loaded the machine-wide copy from `System32`. The receipt therefore targets both
native-module folders, and the installed check rejects any loaded copy outside the package.

## Owner confirmation (recorded 2026-11-27)

The owner recorded these facts for this release. They are recorded as given; product user counts are not treated as the licence's organisation-user counts, and no PC total or affiliate figure is inferred.

| Fact | Recorded value |
| --- | --- |
| Distributor | Intcad development team, **not yet a registered company**. No legal entity name exists. |
| Team size | 4 people |
| Product users | No more than 10 |
| Revenue | None at present |
| Community concurrent users | No more than 5 |
| Distribution route | Closed-source commercial use; no reliance on the open-source, student or academic exceptions |
| Community licensee and runtime redistribution responsibility | PP (confirmed; responsibility accepted) |
| App-local runtime maintenance and technical acceptance | Owner (Huaiyuan Jing), who also owns release decisions |
| Direction | Bundle the required Microsoft runtime in the MSI and portable ZIP; self-contained delivery |

Gaps that could still affect the licence conclusion, held for PP rather than reopened as a questionnaire: the licence's organisation test uses **PCs or users** and **affiliates**, neither of which the recorded facts state; the recorded figures are the team, product-user and concurrent-Community counts only. If the 250-PC-or-user / revenue threshold or the affiliate question later becomes material, PP records the basis. Nothing here substitutes for a company name, an account status or an acceptance record.

PP's role confirmation is the record of responsibility. Specific legal acceptance by end users or downstream distributors is evidenced only by the acceptance mechanism actually shipped and accepted; no such acceptance is claimed yet.


## Distribution terms and acceptance (implemented)

- `packaging/windows/VC-RUNTIME-TERMS.txt` is the end-user text: use only with this Intcad copy,
  no modification or separate distribution, no notice removal, downstream distribution only as
  part of the complete package with the terms, Microsoft as third-party beneficiary, warranty and
  liability limits, export compliance, and acceptance.
- **MSI:** the installer now shows a licence page built from
  `packaging/windows/INTCAD-LICENSE.rtf`; Next is disabled until the terms are accepted. A silent
  install has no page, so it must pass `INTCAD_RUNTIME_TERMS=1`; a launch condition refuses
  otherwise.
- **Portable ZIP:** the terms and receipt are in `mastra-agent\`. On first start of a copy that
  carries the receipt, `crates/ui-egui/src/runtime_terms.rs` shows the same terms once and records
  acceptance for that runtime build (the receipt's file hash) under the app's own data folder.
  Choosing Quit closes the app. A later Intcad release with newer runtime files asks again.
  A run started with `--control` is automation; the operator answers for that machine and the
  install-time terms cover an installed copy, so the first-run window is not drawn there.
- The application's own MIT OR Apache-2.0 licences and all third-party notices are unchanged.
  This adds terms for the Microsoft files only, not a blanket acceptance of the Visual Studio IDE
  licence.

Remaining PP items (specific, one round):

1. Review `VC-RUNTIME-TERMS.txt` and the two acceptance surfaces for 2026 Community §8(b), and
   confirm the wording is adequate or return corrections.
2. Record PP's Community acquisition reference outside the repository (account/licence record);
   this document keeps only the public installer identity and the role assignment.
3. Confirm the final receipt (`packaging/windows/vc-runtime-files.json`) names the intended
   unmodified payload after reviewing the table above. No per-hash approval loop was added: the
   receipt is checked mechanically on every build and every installed/portable verification.
## Open items for PP (concentrated)

The three items above are the only remaining licence actions; the engineering facts (source,
versions, hashes, signature, loader behaviour, acceptance flow) are recorded and mechanically
verified.

## Observed project state (no approval implied)

- The repository's own terms, notices and About page carry the third-party licences but no separate end-user runtime terms; the MSI deliberately skips a licence page because the application itself is MIT OR Apache-2.0.
- No registered company, no company legal name, and no existing acceptance record are present in the repository.

## Required payload receipt

For each DLL, record the stable edition, official installer/channel and toolset identity, exact path inside its installed `VC\Redist\MSVC` tree, matching REDIST clause, PE machine (`AMD64`), file version, validated Microsoft Authenticode signer, SHA-256, and required notice. Record the observed loaded path and matching hash in the installed Node child on clean Windows without a global VC runtime. Repeat against the portable ZIP. The preliminary file list is **`VCRUNTIME140.dll` (import observed; source, version, signature, hash, loader proof and approval pending)**. No `System32`, cache of unrelated products, unofficial download, preview/debug binary, or global install is an input. Do not infer a valid signature or permission from a filename or a matching version string.
