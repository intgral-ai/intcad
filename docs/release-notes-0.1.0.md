# Release notes: 0.1.0 (stable candidate)

Version `0.1.0`, prepared from the `develop` tree for a stable `release/v0.1.0`
candidate. This page is the notes draft, not a publication, tag, signature or
green-gate claim. The final tested checkout SHA and its gate/package/install
evidence are recorded for the fixed candidate; this preparation commit binds
none of them. See [releasing.md](releasing.md) for the production path.

## What changed since `0.1.0-beta.1`

`v0.1.0-beta.1` is the only published Beta tag and its assets stay frozen. The
98 commits that followed through beta.3 and develop include:

- **DFM rule layer**: the INTdfm rule/report port, edge and bend facts, hole
  rules and the full-catalog fixture comparison, with the non-redistributable
  gauge data removed and the public authorization recorded.
- **Kernel**: a multi-lump union now unions by lump, so a disjoint piece is not
  nested as a void (#55).
- **Assistant and packaging**: one page, Linux focus/idle fixes and the
  installable desktop web view; the Mastra runtime remains unapproved and is
  not bundled by the ordinary release path.
- **Agent chat**: direct chat text and tools stream through a bounded reader.
- **CI**: the web-offline check runs in its own bounded job.

Since `0.1.0-beta.3` the 30 non-merge commits are the DFM rule-layer series, the kernel
multi-lump union fix, the Assistant/packaging change and the two CI/chat fixes
above.

## Action matrix and property lane

This candidate does **not** use the Beta exception path. On `develop` at
`b1a789d85`, run `43553634525` shows the action matrix, action-matrix
determinism and the randomized property lane all **succeed**. The stable
`release/v0.1.0` gate reruns them at the final candidate and must see the same
real success; a `release/vX.Y.Z` name never reads
`tests/action_matrix/beta-exceptions.json`.

For history: the Beta.3 promotion did accept a reviewed list (213 strict
baseline pass-to-fail identities and 956/972 unbaselined identities, depending
on the revision). Those are Beta evidence, not a stable waiver, and are not
carried into this release. The baseline was re-blessed on `develop`, and the
property test accepts only the reviewed non-manifold refusals (an exact
Box/Sphere point tangency and the reviewed Cylinder/Sphere seam class) while
grading every other failure through `tests/KNOWN_FAILING_TESTS`. If either lane
fails at the final SHA, this release is blocked rather than excused.

## Packages and signing

Every package is built from the fixed candidate SHA and is **unsigned**, except
the macOS script's existing ad-hoc local signature. macOS is **not notarized**
and Windows has no code signature or SignPath signing yet. No installer, store
or physical-machine run is claimed by this preparation commit.

## Full gate vs Beta gate

- `release/vX.Y.Z` is the **stable** candidate: the `release-gate` workflow
  requires the full `ci`, kernel rig and packages to really succeed.
- `release/vX.Y.Z-beta.N` is a **Beta** promotion: only the itemised
  action-matrix exceptions are accepted; every build and non-kernel job must
  still succeed.

A Beta result, a successful nonpublishing `release.yml` package build, or an
older Beta's evidence cannot authorize the stable candidate. Do not carry an
older Beta's artifact, digest or install result to a new SHA.

## Not done

No tag, GitHub Release, store submission, signing request or branch-protection
change. `main` promotion needs the required `release-gate` to pass at the final
tested SHA.
