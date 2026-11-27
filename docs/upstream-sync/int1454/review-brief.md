# Independent review brief — INT-1454 ledger

You are reviewing the Phase 0 ledger, **not** the implementation. You did not write it. Your job is
to decide whether each `Already Covered` / `Not Applicable` / `Blocked` row is supported by the
evidence it cites, and whether the ledger's coverage claim is true.

## What to check

1. **Coverage.** Every non-merge commit in `06749ac4..v0.44.1` (4,939) must map to at least one
   unit. Re-derive the count yourself; do not trust the table.
   ```sh
   git log --no-merges --format='%H' 06749ac4..v0.44.1 | wc -l
   git log --no-merges --oneline 06749ac4..v0.44.1 -- <unit paths> | wc -l
   ```
   Compare with `generated/commits.tsv`. Report any commit that maps to no unit, and any unit whose
   count cannot be reproduced.

2. **`Not Applicable` rows.** For each, decide whether the reason is a genuine product/environment
   mismatch or an excuse for difficulty. The Spec forbids using "hard to migrate" as a reason.
   Rows to check: X1 (retired paths), X2 (empty commits), A3 (plugins), B1 (solidcore CI jobs),
   B4 (Windows GPO/branding), B8 (screenshots), U3 (brand UI).

3. **`Already Covered` rows.** There are none asserted as complete in this draft; if you find one,
   require a code reference **and** a test.

4. **`Blocked` rows.** Check that the blocker is external (licence, missing prerequisite) and not
   a choice: U3 brand crates, D5 STEP until K1, A6 default changes.

5. **Numbers.** Spot-check at least six figures against git: the two SHA pins and their distance,
   the format blob identity, the 339→7 schema backlog, the 582→636 command count, the brand-path
   file counts, the conflict matrix counts.

6. **Claims about today's IntCAD.** Check `security-findings.md` SEC-1 and SEC-2 against the code;
   these drive a proposed standalone security PR.

## How to report

For each row you checked, output: row id, verdict (`supported` / `unsupported` / `needs evidence`),
the command and output that decides it, and one sentence. End with:

- rows that must change classification before Phase 0 can pass;
- anything the ledger omits entirely;
- your overall verdict: **Phase 0 gate passes / fails**, with the blocking items listed.

Do not edit files. Do not run builds. Read-only git and file inspection only. Quote the command
output you relied on.
