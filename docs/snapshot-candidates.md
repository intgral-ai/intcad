# Design snapshots and candidates

`document.snapshot.capture` freezes the live design, its manifest and every supported dependency
into one immutable candidate that a validation run can use while the user keeps editing. It is a
normal command (`Session::execute`), so scripts, the control channel and MCP reach it the same
way. The guarded interfaces (control and MCP) admit it: a stale `expectedDocument` is refused with
`STALE_DOCUMENT` before a candidate is written, and a current stamp returns `documentStamp` equal
to the captured source stamp.

In the desktop app the control path answers a capture on a scratch copy off the window thread, so
a large container is not written on a frame. That copy keeps the caller's design path, so relative
design links resolve from the design's folder rather than the process working directory.

## Call and reply

- Input: `{}` (no parameters). The guarded executor also returns `documentStamp`, equal to the
  captured source stamp.
- Reply: `{candidate: CandidateRef}`. `CandidateRef.source.revision` is a canonical decimal
  `u64` string, never a JSON number. Digests are `sha256:<64 lowercase hex>`.

Capture reads the document only: it does not evaluate the model, mark the design saved or add an
undo step. It leaves the live visibility and uncaptured pending moves untouched. The snapshot is
the design that a save would persist (including current visibility); uncaptured pending moves are
not part of it. A captured design reopens independently and keeps its parameters and feature
history.

## Container

The host store is `SOLIDCRAFT_CANDIDATE_DIR` or `solidcraft_engine::cmd::set_candidate_store` when
set, else `validation-candidates/` under the per-user data folder. A candidate is:

- `<candidateId>.zip` — immutable, published with a create-only atomic write. Entries are
  `manifest.json` and exactly the manifest's inputs (the design input is always `design.sc`, plus
  frozen dependencies).
- `<candidateId>.json` — the durable lookup record: artifact, candidate and manifest digests.
- `<candidateId>.pin` — an active validation reference.

The manifest identifies the exact persisted bytes of each input. `candidateDigest` is SHA-256 of
the RFC 8785 (JCS) encoding of `{formatVersion, engineVersion, inputs}`; `manifestDigest` is
SHA-256 of the JCS encoding of the complete manifest; `artifact.digest` hashes the exact container
bytes. Neither digest is stored inside the manifest. `capturedAt` is RFC3339 UTC with exactly
milliseconds, and `manifest.json` is reserved (it can never be an input path).

## Dependencies

Insert Design links are frozen: the linked design's bytes become a `component` input under
`inputs/`, the dependency gets a stable `dependencyId`, and the captured design's link is
rewritten to that relative path. A link that cannot be read, changed since this design last read
it, has no stable identity, or itself links to another design fails with `DEPENDENCY_CHANGED`
instead of keeping a mutable live path. A sketch text whose font is a font *file* is a mutable
external path too: 1.0 refuses it with `INVALID_CANDIDATE` rather than retaining it. Other
unsupported captures fail with `INVALID_CANDIDATE`; diagnostics do not echo absolute local paths.


## Limits and retention

The contract §4 ceilings apply: 1 GiB of persisted input, 256 inputs, a 1 MiB manifest. Completed
unpinned candidates are retained for seven days (`solidcraft_io::candidate::RETENTION`). Cleanup
never removes a pinned candidate, so an active validation job cannot lose its input; store
pressure refuses a new capture rather than deleting active evidence.
