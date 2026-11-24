# v0.1.3 release handoff (for the signing and publication approvals)

Status: **pre-candidate; five integration PRs merged, two open.** This page is the concise handoff that goes to the approving
owner. It is updated as facts land; a row that has not been run says NOT RUN, and nothing
here is a publication authorization.

## Identity

| Item | Value |
|---|---|
| Version | 0.1.3 (workspace `Cargo.toml`, our own `Cargo.lock` entries; `cbc`/`dpi` stay third-party 0.1.2) |
| Release notes | `docs/release-notes-0.1.3.md` (the only notes path the publisher accepts) |
| Design format | folded to format 51 by `scripts/fold-format-pending.sh` |
| Candidate SHA | **NOT FROZEN** — must be the tested merge/group SHA of the `release/v0.1.3` → `main` promotion PR |
| Promotion branch | `release/v0.1.3` (local `994b280dda`, to be rebased onto the merged `develop`) |

## Merged pull requests (develop)

| PR | Squash merge |
|---|---|
| #226 — validation race fix | `e6ba5b1bfd` |
| #231 — bounded `apt-get update` in CI | `70e0af0ae7` |
| #221 — Assistant at the bottom centre, native shape | `1d523a54a7` |
| #230 — signed updates and draft-only publisher | `e8c4b1be2d` |
| #229 — slash commands in the Assistant | `f18a9aec27` |

`develop`'s full CI on `f18a9aec27` succeeded. Still open: **#214** (packages carry the
built-in agent; rebased as `d4819c6a9d`, CI running) and **#224** (login; CI on `5776381321`).
The promotion PR **#237** (`release/v0.1.3` → `main`) exists as a draft and has not started the
formal gate.

## Gates

| Gate | State |
|---|---|
| Required CI (`ci` on develop, `release-gate` on main) | running on the open PRs; `develop`'s gate was red on #225 and is fixed by #226 |
| Three-platform production packaging | NOT RUN — comes from the formal Release Gate's 14 package/evidence pairs |
| Linux install and launch | integration preview only: deployed binary in the `i3int` VM, 8/8 steps (not an installer) |
| Windows install and launch | earlier rehearsal only, not this candidate |
| macOS install and launch | **NOT TESTED — no Apple hardware** |
| Production login / Gate B | pending the Login Agent's staging evidence |
| Licences and notices | inherited rehearsal audit; the actual candidate package inventory is not yet inspected |
| Package hygiene | rehearsal tarball/ZIP inspected, candidate not yet |
| Minisign | production key embedded and client verification fails closed; candidate signing not yet run |

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

## Publication approval

Final policy (owner decision): multiple authorized approvers, **one approval is sufficient**.
`Huaiyuan-Jing` and `paipeline` may each approve independently, including a workflow they
triggered themselves; admin bypass stays disabled and no agent may approve on a human's
behalf. A Linear comment counts only when it names the **version and the candidate SHA**
and approves publication of the verified draft; anything less is not authorization.

## Open blockers

1. CI runs for the open release PRs are still in progress.
2. #223's wasm/IME/byte-cap fixes and the Migration Agent's remote-tag check are in progress.
3. Gate B needs real staging credentials and evidence from the Login Agent.
4. macOS hardware is unavailable, so that platform cell stays NOT TESTED.
5. `RELEASE_AGENT_LEGAL_APPROVED` / `RELEASE_AGENT_INSTALLED_APPROVED` are owner actions.
