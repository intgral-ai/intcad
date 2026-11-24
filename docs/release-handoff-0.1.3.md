# v0.1.3 release handoff (for the signing and publication approvals)

Status: **pre-candidate.** This page is the concise handoff that goes to the approving
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

## Merged pull requests

To be filled from the actual merge history when the candidate is frozen. Planned order:
#226 (validation race), #214 (packages carry the agent), #224 (login), the Migration
Agent's signed-distribution PRs, #221 (Assistant), #223 (slash commands).

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

Before dispatch, the owner must set, for the exact candidate, in the repository/environment:
`RELEASE_AGENT_LEGAL_APPROVED` and `RELEASE_AGENT_INSTALLED_APPROVED` as
`<source SHA>:<run ID>:<attempt>`.

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
