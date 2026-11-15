# PIL-291 acceptance evidence (archive branch)

Not for merge. This branch keeps the PIL-291/DFM acceptance record and its fixtures out of the
throwaway `worktrees/acceptance/` folder so reviewers can still fetch it:

- `PIL291-FINAL.md` — the eight-ticket delivery table and merge recommendation.
- `PIL291-ACCEPTANCE.md` — desktop runs: Chinese phrase, model tool entry, row/marker linkage,
  preview accept/reject/stale counts, the automatic-check switch defect and its fix.
- `IME-RESULT.md`, `PLACEMENT-INT-1472.md`, `INTEGRATION.md` — input-method and placement notes.
- `evidence/` — page screenshots of the report card, the preview card and the accepted state.
- `fixtures/hole-tier/` — the Hole Tier design with its `.intcad/` project, check runs and
  reports, so a run id in the record can be opened and compared.
- `drive.py`, `bridge.py`, `compare_card.py` — the scripts used to drive the app's control
  channel and to compare the card with `report.json`/`findings.json`.
- `TEST-ONLY-shop-two-tier.json` — the two-tier test shop data used by the fixture.

`candidate2` and `pil305` lines in the documents refer to the build trees
`coord/pil-291-candidate2` and `coord/pil-305-preview-integration`; the wrappers are Nix store
paths that may be garbage-collected, so the run reports here are the durable record.
