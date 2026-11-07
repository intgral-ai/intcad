# Design snapshots and candidates

`document.snapshot.capture` freezes the live design, its manifest and every supported dependency
into one immutable candidate that a validation run can use while the user keeps editing. It is a
normal command (`Session::execute`), so scripts, the control channel and MCP reach it the same
way; guard it with the execution envelope's `expectedDocument` so a stale document stamp fails
before anything is pinned or written.

## Call and reply

- Input: `{}` (no parameters). The guarded executor also returns `documentStamp`, equal to the
  captured source stamp.
- Reply: `{candidate: CandidateRef}`. `CandidateRef.source.revision` is a canonical decimal
  `u64` string, never a JSON number. Digests are `sha256:<64 lowercase hex>`.

Capture reads the document only: it does not evaluate the model, mark the design saved, add an
undo step, or change visibility or uncaptured moves. A captured design reopens independently and
keeps its parameters and feature history.

## Container

The host store is `SOLIDCRAFT_CANDIDATE_DIR` when set, else `validation-candidates/` under the
per-user data folder. A candidate is:

- `<candidateId>.zip` — immutable, published with a create-only atomic write. Entries are
  `manifest.json` and exactly the manifest's inputs (one `design.sc`, plus frozen dependencies).
- `<candidateId>.json` — the durable lookup record: artifact, candidate and manifest digests.
- `<candidateId>.pin` — an active validation reference.

The manifest identifies the exact persisted bytes of each input. `candidateDigest` is SHA-256 of
the RFC 8785 (JCS) encoding of `{formatVersion, engineVersion, inputs}`; `manifestDigest` is
SHA-256 of the JCS encoding of the complete manifest; `artifact.digest` hashes the exact container
bytes. Neither digest is stored inside the manifest.

## Dependencies

Insert Design links are frozen: the linked design's bytes become a `component` input under
`inputs/`, the dependency gets a stable `dependencyId`, and the captured design's link is
rewritten to that relative path. A link that cannot be read, changed since this design last read
it, has no stable identity, or itself links to another design fails with `DEPENDENCY_CHANGED`
instead of keeping a mutable live path. Unsupported captures fail with `INVALID_CANDIDATE`.

## Limits and retention

The contract §4 ceilings apply: 1 GiB of persisted input, 256 inputs, a 1 MiB manifest. Completed
unpinned candidates are retained for seven days (`solidcraft_io::candidate::RETENTION`). Cleanup
never removes a pinned candidate, so an active validation job cannot lose its input; store
pressure refuses a new capture rather than deleting active evidence.
