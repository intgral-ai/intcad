# v0.1.3 release handoff (for the signing and publication approvals)

Status: **pre-candidate; six PRs merged, the paid-authority chain and the tag-push gate open.** This page is the concise handoff that goes to the approving
owner. It is updated as facts land; a row that has not been run says NOT RUN, and nothing
here is a publication authorization.

## Identity

| Item | Value |
|---|---|
| Version | 0.1.3 (workspace `Cargo.toml`, our own `Cargo.lock` entries; `cbc`/`dpi` stay third-party 0.1.2) |
| Release notes | `docs/release-notes-0.1.3.md` (the only notes path the publisher accepts) |
| Design format | folded to format 51 by `scripts/fold-format-pending.sh` |
| Candidate SHA | **NOT FROZEN** — must be the tested merge/group SHA of the `release/v0.1.3` → `main` promotion PR |
| Promotion branch | `release/v0.1.3`: `c929f797e1` (version 0.1.3, notes, folded format 51), `fa6582f8c9` (widened notes), `cc4f68fdad` (FreeBSD limit); to be rebased onto the final `develop` |
| `Cargo.lock` at promotion | must be regenerated with the version commit: `solidcraft-account-native` and `solidcraft-suspend-clock` move `0.1.2 → 0.1.3` (two lines); measured, `cargo metadata --locked` then passes |

## Merged pull requests (develop)

| PR | Squash merge |
|---|---|
| #226 — validation race fix | `e6ba5b1bfd` |
| #231 — bounded `apt-get update` in CI | `70e0af0ae7` |
| #221 — Assistant at the bottom centre, native shape | `1d523a54a7` |
| #230 — signed updates and draft-only publisher | `e8c4b1be2d` |
| #229 — slash commands in the Assistant | `f18a9aec27` |
| #236 — derived packages record their own artifact | `085675557d` |

`develop` is `97236b5dd7` (it also carries #236 and #238) and its full CI succeeded.
Two merges remain, then the promotion:

| Merge | Head | State |
|---|---|---|
| #214 — packages carry the built-in agent | `e65d647201` | full CI running (17/20, 0 failures); it also restores the `npm/Node` policy anchor its gate run caught missing |
| **one Integration PR**: #224 + #227 + #233 + #235 + #234 + #239 | candidate `eade4198c2` built and locally verified; will be re-based on the post-#214 `develop` and pushed as `integration/v0.1.3` | one full CI is the merge gate; the original PRs stay open as review records and are closed with a pointer afterwards |
| promotion `release/v0.1.3` → `main` (#237) | three rehearsed commits: version 0.1.3 + notes + folded format 51, widened notes, FreeBSD known limit, with the two-line `Cargo.lock` refresh folded into the version commit | draft; runs the required `release-gate` once marked ready |

The promotion PR **#237** (`release/v0.1.3` → `main`) is a draft and has not started the formal
gate.

## Gates

| Gate | State |
|---|---|
| Required CI (`ci` on develop, `release-gate` on main) | running on the open PRs |
| Three-platform production packaging | NOT RUN — comes from the formal Release Gate's 14 package/evidence pairs (§18 of the readiness report lists every asset name) |
| Linux install and launch | integration preview only: deployed binary in the `i3int` VM, 8/8 steps (not an installer) |
| Windows install and launch | earlier rehearsal only, not this candidate |
| macOS install and launch | **NOT TESTED — no Apple hardware** |
| Production login / Gate B | prerequisites ready, acceptance NOT RUN: `staging` environment allows `main`, staging service preflight PASS (`/healthz`, `/readyz`, `/v1/status` with `min_client=0.1.3`, six authorizations `remaining=299`, 0.1.2 → `VERSION_UNSUPPORTED`), two staging accounts verified (`…+intcad-staging@`, `…+intcad-gateb-switch@`) |
| Candidate account tuple | production variable set and validated; expected identity `sha256 89abe5c9…f41bb`; CI proves the Linux build output only, so each packaged binary is verified at acceptance (§17 of the readiness report) |
| Licences and notices | inherited rehearsal audit; the actual candidate package inventory is not yet inspected |
| Package hygiene | rehearsal tarball/ZIP inspected, candidate not yet |
| Minisign | production key embedded and client verification fails closed; candidate signing not yet run |

Known limits recorded in the release notes: Windows Authenticode / Apple signing are out of scope;
an installed 0.1.2 cannot update itself; the FreeBSD package carries no account configuration, so
sign-in and paid operations are unavailable there.

## Signing

The `release-signing` environment has two authorized reviewers (`Huaiyuan-Jing`,
`paipeline`) with self-review allowed; admin bypass is disabled. The workflow creates a
**draft only**, verifies every uploaded asset and both checksum lists against the formal
gate's accepted bytes, and signs `SHA256SUMS.txt`.

Approval link: **not created yet** — it exists after `publish-release.yml` is dispatched
with the frozen tag, `source_sha`, gate `run_id`, `run_attempt` and `notes_file`.

Before dispatch, for the exact candidate, an authorized human sets in the repository
environment: `RELEASE_AGENT_LEGAL_APPROVED` and `RELEASE_AGENT_INSTALLED_APPROVED`,
each `<source SHA>:<run ID>:<attempt>`. These must **not** be set in advance: they record
the actual rights and installed-agent review of that candidate. The release manager posts
the exact names, values, location and steps in INT-1503 and mentions `@p.paipeng.p` once
the candidate and its acceptance evidence exist; PP or the CTO performs the review.

The Minisign key, password, GitHub App and environment secrets are already configured;
their live state is confirmed with the Migration Agent rather than requested again.

**Production account tuple (a gate prerequisite only the owner can set).** On #224's branch the
formal gate passes `production_account_config: ${{ vars.INTCAD_PRODUCTION_ACCOUNT_CONFIG }}` into
`release.yml`, which refuses to build a candidate when that input is empty ("a candidate is never
built with an anonymous account"). The repository variable must hold the JSON tuple
`{"INTCAD_WORKOS_PUBLIC_CLIENT_ID": ..., "INTCAD_AUTHKIT_VERIFICATION_ORIGIN":
"https://<host>", "INTCAD_PRODUCT_HTTPS_ORIGIN": "https://<host>"}`. It is public configuration,
not a secret, and must not be committed. `scripts/ci/verify_embedded_account.py` verifies the
built binary embeds exactly that tuple without printing secrets. As of 2026-11-24T17:40Z the repository variable **is set** and validates: three keys,
`client_` + 33 alphanumerics, AuthKit origin `https://cuddly-globe-52.authkit.app`, product origin
`https://api-production-02512b4a.up.railway.app`, both bare HTTPS origins. The candidate still has
to prove the same tuple is embedded in its binaries (`verify_embedded_account.py`).
Expected account identity for that check: **sha256
`89abe5c94fb1fbaa608e0763224f4c9e241fee7087c3bd6ee4da96c3a75f41bb`** (canonical JSON of the three
variable values, sorted keys, no spaces).

## Publication approval

Final policy (owner decision): multiple authorized approvers, **one approval is sufficient**.
`Huaiyuan-Jing` and `paipeline` may each approve independently, including a workflow they
triggered themselves; admin bypass stays disabled and no agent may approve on a human's
behalf. A Linear comment counts only when it names the **version and the candidate SHA**
and approves publication of the verified draft; anything less is not authorization.

## Release sequence is forced by workflow registration

`workflow_dispatch` only accepts a workflow that exists on the **default branch** (`main`),
measured 2026-11-24T17:45Z:

```
main:  release-gate.yml present (18263 B), release.yml present (31275 B)
       release.yml on main declares only `beta_three_platforms` (no account inputs)
       publish-release.yml  absent
       staging-packages.yml absent
develop: publish-release.yml present (5264 B), staging-packages.yml absent
```

So `Package builds (nonpublishing)` on `main` cannot start a staging build either: its `main`
copy accepts no `staging_account_config` input. Consequences:

1. Gate B (staging installation and sign-in acceptance) **cannot be dispatched before #237
   merges into `main`**; the merged promotion carries `staging-packages.yml` and the account
   inputs, after which `Staging desktop packages (nonpublishing)` becomes dispatchable.
2. `publish-release.yml` (draft-only publisher, from #230) likewise becomes dispatchable only
   after #237 is on `main`. The signing approval therefore necessarily follows the promotion,
   the formal gate and Gate B — never before them.
3. This is a GitHub registration rule, not a gate we may bypass: no local or ad-hoc build may
   substitute for the Gate B run.

## Open blockers

1. #214's full CI is still running; #224 waits for the new `develop`; #239 and the integration PR
   follow it.
2. Gate B staging packages cannot be dispatched until #237 is on `main`; its acceptance cells are
   all NOT RUN until then.
3. macOS hardware is unavailable, so that platform cell stays NOT TESTED.
4. `RELEASE_AGENT_LEGAL_APPROVED` / `RELEASE_AGENT_INSTALLED_APPROVED` are owner actions, set only
   once the candidate and its evidence exist.
5. The formal post-promotion gate is the **tag push** (#239): a PR gate's SHA is a synthetic merge
   ref that never lands on `main`, so the publisher can only consume a run produced at the tagged
   `main` commit.
