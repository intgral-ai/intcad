# Local validation port

`ValidationPort` runs versioned checks on an immutable `CandidateRef` captured by
`document.snapshot.capture` (#11). It is the host-side implementation of
[Mastra contract §9](mastra-runtime.md#9-validationport) and is vendor-neutral: the port
admits, persists and reports jobs, while a `Runner` computes the checks. The built-in
runner is local and headless; a remote CI adapter can implement the same trait later
without changing the port, the records or the report shape.

```text
submit(candidate, profile, requestId) -> runId
status(runId) -> state
report(runId) -> ValidationReport
cancel(runId)
```

Rust: `solidcraft_engine::validation::ValidationPort`. CLI: `solidcraft-cli validate`.

## Profiles

`native-basic` version `1.0.0` is built in and always requires:

| Check | What it proves |
|---|---|
| `input.integrity` | The container, manifest, every input digest and the supplied `CandidateRef` agree. |
| `native.rebuild` | The captured design opens and evaluates with no feature, parameter, placement or unresolved-dependency error. |

Task profiles add `parameters.required` and/or `geometry.exact` with the §9.1 shapes.
The port freezes the complete profile at admission. The runner cannot omit the native
checks, mark them optional, change their parameters or widen a tolerance: such a
profile is refused with `PROFILE_UNSUPPORTED`.

`ProfileRef.digest` is SHA-256 of the RFC 8785 JCS encoding of the complete profile
object. Submission equality compares JCS bytes of `{candidate, profile}`; whitespace,
key order and the transport's own request ID do not change it.

A geometry requirement is supported only when its `command` is one of the read-only,
exact-geometry commands `inspect.measure`, `model.faces`, `model.edges` or
`document.inspect`. Any other command is recorded as `unsupported`, which can never
become a pass. Display meshes are never used.

## Skill-facing path (#65)

`solidcraft_engine::validation::validate_skill_snapshot` is the UI-independent call path used by
the native Chat skill and by a headless test or CLI. It registers the host-owned profile, submits
the exact `CandidateRef`, runs the local worker and returns a `SkillValidationReport`. The report
reference names:

- the snapshot (`snapshotId`, candidate and manifest digests);
- the captured document (`documentId`, `documentRevision`);
- the profile (`profileId`, `version`, `digest`);
- the runner (`runnerId`, `version`, `engineVersion`);
- the required checks and each check's state;
- the exact persisted report bytes (`report` artifact id, digest and size).

The Beta Text-to-CAD profile is `text-to-cad` version `1.0.0`. It always requires
`input.integrity`, `native.rebuild`, `parameters.required` and `geometry.exact`; a caller cannot
rename, omit or weaken them. Its digest is computed from the complete profile, including the
task's parameter and measurement requirements.

The request's `requiredChecks` must equal the profile's required check ids exactly. A caller cannot
narrow coverage by dropping a check, or widen it by naming a check the profile did not require; both
are refused with `PROFILE_UNSUPPORTED` before a job is admitted.

The report separates `snapshotPassed` from `approved`. `snapshotPassed` means the exact captured
snapshot completed every required check under the frozen profile. `approved` also requires the
caller to supply the current live `documentId` and `revision`; omitting the live document can never
claim approval. A report for snapshot A never approves snapshot B. If a manual parameter change moved
the revision after capture, the passing report stays historical evidence, `snapshotPassed` remains
true, and `approved` is false. Re-capture the new revision and submit it again before showing a new
validated result.

`failure` is the structured terminal reason, distinct from the prose `reason`:
`kind` is `timeout`, `infrastructure_error`, `quality_failed`, `cancelled` or `unknown`, and `code`
retains the persisted contract code when there is one. A timeout is `kind: timeout` with
`code: TIMEOUT`; it is not collapsed into a generic `INTERNAL_ERROR` or a quality failure.

The skill descriptor from #64 has its own schema version `0.1.0`. That is the descriptor format
version, not the validation profile version. The Beta validation profile here is `text-to-cad`
`1.0.0`; the two versions are independent and must not be substituted for each other.

## Runner

The local runner:

1. Reopens the candidate through `solidcraft_io::candidate::lookup`, which verifies the
   container, manifest and every input digest, and compares the result with the supplied
   `CandidateRef`.
2. Copies the exact input bytes into a private scratch directory, so the captured design's
   relative links resolve to captured inputs and never to the live filesystem.
3. Opens the design in a fresh `Session` with the normal native command path. The live
   document is not read, changed or saved.
4. Runs the checks in profile order, after `native.rebuild` has produced the isolated
   session, and enforces the execution deadline and cancellation marker before each step.

Evidence is JSON produced by the runner. The port assigns each artifact an identity,
persists its exact bytes and records the digests. A passing required check must have at
least one evidence artifact whose digest and size verify at report time.

## States and persistence

`queued -> running -> passed | quality_failed | infrastructure_error | cancelled | unknown`

- A required geometric or parameter mismatch is `quality_failed`.
- Worker failure, timeout, malformed output, a missing/unsupported check or an
  unverifiable required evidence artifact is `infrastructure_error`.
- Cancellation persists `runs/<runId>.cancel` before any signal. It wins before the
  terminal commit, so a late passing result cannot overwrite it.
- A host restart never restarts an admitted job. A committed report is verified and
  adopted before its terminal state is published; a tampered or unverifiable report is
  not adopted. A lost or ambiguous worker result becomes `unknown`; a queued job that
  never started becomes `infrastructure_error`.
- A terminal state is immutable and is never paired with a report for another state. A
  second process that commits the same terminal state keeps the first report: the commit
  is idempotent, so a cancelled submit and a cancel do not race into `REPORT_MISMATCH`.
  If a terminal run's report is missing or corrupt, `status` verifies availability before
  reporting it (read-only observers included), and `report()` returns `NOT_READY`.
  A worker reconcile removes the unverifiable report, keeps the terminal state and
  persists `reportAvailable: false`; it does not rewrite `passed` as `unknown`.
- A recovered queued run is pinned again when a worker claims it. If its candidate
  inputs are gone, the run becomes `infrastructure_error` instead of starting.
- Reconciliation only touches workers owned by this host or whose recorded host process
  is no longer alive. `status`, `report` and `cancel` open the port read-only: a second
  process observing a live submit never turns it into a restart. Cancel still writes the
  durable marker and finalizes the run as `cancelled`.

All five terminal states are immutable. The store keeps the admission, frozen profile,
worker identity, cancellation marker, report and state as separately atomically written
files under the validation directory:

```text
runs/<runId>.json          admission
runs/<runId>.profile.json  frozen complete profile
runs/<runId>.worker.json   worker identity, pid and deadline
runs/<runId>.state.json    current or terminal state
runs/<runId>.cancel        cancellation marker
runs/<runId>.report.json   exact persisted report bytes
keys/<sha256>.json         idempotency mapping
evidence/<runId>/...       digest-checked evidence
```

Reports bind `runId`, candidate ID and digests, manifest digest, `ProfileRef`, the
runner identity (`runnerId`, `version`, `engineVersion`) and the terminal state. They
contain exactly one `CheckResult` per profile check. The report reference hashes the
exact persisted report bytes.

## Bounds

The defaults are the contract's §4 ceilings: 2 workers and 8 waiting jobs, 600 s of
execution, 300 s in the admission queue, a 4 MiB report, 128 checks, 64 evidence
artifacts of at most 16 MiB each and 64 MiB aggregate per run, 10,000 job records and
7-day retention. Active jobs pin their candidate inputs with a per-run pin; admission is refused with
`RESOURCE_EXHAUSTED` rather than evicting active evidence. The admission and queued state
are written before the pin, and cleanup also removes a pin left by a crash before the
admission became visible. `cleanup(now)` removes completed records and their unpinned
candidate inputs only after retention, and never touches an active job.

## CLI

```sh
solidcraft-cli validate submit request.json --profile task.json \
    --candidate-dir candidates --validation-dir validation
solidcraft-cli validate skill request.json --profile task.json \
    --candidate-dir candidates --validation-dir validation --json-out skill-report.json
solidcraft-cli validate status <runId> --validation-dir validation
solidcraft-cli validate report <runId> --validation-dir validation --out report.json
solidcraft-cli validate cancel <runId> --validation-dir validation
```

`request.json` is `{candidate, profile, requestId}` where `requestId` is
`<unix-ms>:<uuid-v4>`; `profile` is a `ProfileRef`, and its digest may be omitted when
the request names the `--profile` file. `submit` runs the local worker to a terminal
state and prints `status`; a quality failure is a successful run with
`"state": "quality_failed"`, not a CLI error. `native-basic` needs no `--profile`.

`skill` is the UI-free headless consumer of the skill-facing path. Its request also carries
`live: {documentId, revision}` and it requires the complete `text-to-cad` profile with
`--profile`. It prints the bound `SkillValidationReport`, including `snapshotPassed`,
`approved`, `failure` and the persisted report artifact, so a CI job or script can consume the
same fields as a panel without a Chat dependency. A passing snapshot with a missing or moved live
document still prints a report, with `approved: false`.

`status`, `report` and `cancel` open the store read-only and never reconcile a live
worker owned by another process.

The directories default to `SOLIDCRAFT_CANDIDATE_DIR` (or the per-user
`validation-candidates` directory) and `SOLIDCRAFT_VALIDATION_DIR` (or the per-user
`validation` directory).

## Verified turn outcomes (#13)

The Mastra workflow captures a candidate only after its native stage checks pass,
then submits it to the same `ValidationPort` under a new durable key. The runtime
polls `status`, reads `report`, and checks the run/candidate/manifest/profile/
runner/check identity before proposing `validated_success`. A quality failure
returns to planning with a bounded repair and a new candidate/run; infrastructure
errors, timeouts, cancellation and unknown outcomes are separate and never edit
geometry.

The Rust host is the final gate. The `validation.submit/status/report/cancel`
callbacks route to this port, and the host caches the exact report reference it
received. Before publishing `validated_success` it requires all of:

1. the run was submitted by the active turn and uses the host-frozen profile;
2. the host's cached report reference exactly equals the runtime's proposal;
3. the report is `passed`, has no error, and its run/candidate/digest/profile/
   runner/engine values match the admission record;
4. the report covers exactly the profile's checks, every required result is
   `passed`, and each has at least one digest-checked evidence reference;
5. the turn is active and the current live `DocumentStamp` equals the captured
   candidate's source stamp.

A report that passes after the live design changed is retained as history and
shown with its snapshot, report reference and `liveChanged: true`; it is never
approval of the new content. A cancelled turn is not promoted by a late report.

## Not in this port

A remote CI vendor adapter, deployment, a comprehensive production check suite and an
adoption gate are follow-up integrations. The port does not claim those, and it does not
depend on any particular remote vendor.
