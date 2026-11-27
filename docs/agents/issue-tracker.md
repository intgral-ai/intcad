# Issue tracker: Intcad

Issues and specs for Intcad live as GitHub Issues in `intgral-ai/intcad`. Use the `gh` CLI for all operations.

**This file covers the Intcad fork only.** Upstream SolidCraft (`tangency-labs/solidcraft`) keeps its own issues and its own rules; do not read or write them from these instructions. Work that the repository's contributing rules route upstream follows `skills/solidcraft-contributing/SKILL.md` instead.

**Scope every call explicitly:** pass `--repo intgral-ai/intcad` to every `gh issue` and `gh pr` call, and start every `gh api` path with `repos/intgral-ai/intcad/`. Don't rely on whichever remote the checkout you happen to be in points at.

## Conventions

- **Create an issue**: `gh issue create --repo intgral-ai/intcad --title "..." --body-file <file>`. Use a file for multi-line bodies.
- **Read an issue**: `gh issue view <number> --repo intgral-ai/intcad --json number,title,body,state,labels,comments`.
- **List issues**: `gh issue list --repo intgral-ai/intcad --state open --json number,title,body,state,labels,comments`, adding `--label` / `--state` filters as needed; use `jq` to shape the result.
- **Comment**: `gh issue comment <number> --repo intgral-ai/intcad --body "..."`.
- **Apply / remove labels**: `gh issue edit <number> --repo intgral-ai/intcad --add-label "..."` / `--remove-label "..."`. Only apply labels that exist in the repository.
- **Close**: `gh issue close <number> --repo intgral-ai/intcad --comment "..."`.

## Pull requests as a triage surface

**PRs as a request surface: no.** This flag configures the Matt engineering skills only: `/triage` does not treat external PRs as feature requests here. It does not change how ordinary PRs are reviewed, labelled or triaged by maintainers.

If this flag ever becomes `yes`, PRs run through the same labels and states with the `gh pr` equivalents, always with `--repo intgral-ai/intcad`: `gh pr view <number> --comments`, `gh pr diff <number>`, `gh pr list --state open --json number,title,body,labels,author,authorAssociation,comments` filtered to `authorAssociation` of `CONTRIBUTOR`, `FIRST_TIME_CONTRIBUTOR` or `NONE` (drop `OWNER`/`MEMBER`/`COLLABORATOR`), and `gh pr comment`, `gh pr edit --add-label`/`--remove-label`, `gh pr close`.

GitHub shares one number space across issues and PRs, so a bare `#42` may be either: resolve with `gh pr view 42 --repo intgral-ai/intcad` and fall back to `gh issue view 42 --repo intgral-ai/intcad`.

## When a skill says "publish to the issue tracker"

Create a GitHub issue in `intgral-ai/intcad`.

## When a skill says "fetch the relevant ticket"

Run `gh issue view <number> --repo intgral-ai/intcad --json number,title,body,state,labels,comments`.

## Wayfinding operations

Used by `/wayfinder`. The **map** is a single issue with **child** issues as tickets.

**Check the labels before using them.** Run `gh label list --repo intgral-ai/intcad --search wayfinder`; it must show `wayfinder:map` and every `wayfinder:<type>` you plan to use (`research`, `prototype`, `grilling`, `task`). If one is missing, stop and ask a maintainer to add it. These instructions never create labels.

- **Map**: one issue labelled `wayfinder:map` holding the Notes / Decisions-so-far / Fog body: `gh issue create --repo intgral-ai/intcad --title "<map title>" --body-file <file> --label wayfinder:map`.
- **Child ticket**: create the issue, then attach it to the map as a GitHub sub-issue: `gh api --method POST repos/intgral-ai/intcad/issues/<map>/sub_issues -F sub_issue_id=<child-db-id>`. `<child-db-id>` is the child's numeric **database id** (`gh api repos/intgral-ai/intcad/issues/<child> --jq .id`), not its `#number` or `node_id`. Where sub-issues aren't available, add the child to a task list in the map body and start the child body with `Part of #<map>`. Labels: `wayfinder:<type>`. Once claimed, the ticket is assigned to the driving dev.
- **Blocking**: GitHub's native issue dependencies, the canonical, UI-visible edge: `gh api --method POST repos/intgral-ai/intcad/issues/<child>/dependencies/blocked_by -F issue_id=<blocker-db-id>`, where `<blocker-db-id>` is the blocker's numeric **database id** (`gh api repos/intgral-ai/intcad/issues/<n> --jq .id`), not its `#number` or `node_id`. Where dependencies aren't available, start the child body with `Blocked by: #<n>, #<n>`. A ticket is unblocked when every blocker is closed.
- **Frontier query**: enumerate the map's open children with `gh api repos/intgral-ai/intcad/issues/<map>/sub_issues --jq '[.[] | select(.state == "open") | .number]'`, or from the map body's task list where sub-issues aren't available. For each candidate, check its blockers per issue: `gh api repos/intgral-ai/intcad/issues/<n>/dependencies/blocked_by --jq '[.[] | select(.state == "open") | .number]'`. Where that endpoint is unavailable, read the `Blocked by:` line in the child body. If neither source can be checked, leave the ticket off the frontier and say so; never treat missing metadata as unblocked. Drop tickets with an open blocker or an assignee. First in map order wins.
- **Claim**: implementation tickets follow this repository's claim rules in `skills/solidcraft-contributing/SKILL.md`: claim with a draft PR, never by assignment alone. Check for an existing claim with `gh pr list --repo intgral-ai/intcad --state open --search "#<n>"`; if none, branch from the latest `origin/develop`, commit `claim: #<n>` (`git commit --allow-empty -m "claim: #<n>"`), push, and open a draft PR titled `<summary> (#<n>)`. The lowest-numbered open PR for an issue wins. The bot records the claim by assigning the issue; that assignment is not the claim. Comment `/claim` only to stake a claim before you have a branch. Research, prototype or grilling tickets that produce no PR have nothing to claim with a draft PR: confirm with a maintainer, then `gh issue edit <n> --repo intgral-ai/intcad --add-assignee @me` as the session's first write.
- **Resolve**: `gh issue comment <n> --repo intgral-ai/intcad --body "<answer>"`, then `gh issue close <n> --repo intgral-ai/intcad`, then append a context pointer (gist + link) to the map's Decisions-so-far.
