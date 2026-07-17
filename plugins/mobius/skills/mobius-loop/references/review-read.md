# Formal Review read

Load this recipe only after a fresh targeted read shows the selected Objective is `Reviewing`.

Freeze both heads and the root Packet identity from `objective_projection`. Key the inspection
closure by immutable exact Packet, Decision, and Evidence identities. Read the root
`review_packet`, verify its embedded identity, and read every distinct identity declared by its
`evidence_set` and `context.dependency_proofs`.

For each previously unseen dependency Decision, read that exact `review_decision`, verify its
embedded identity, follow its exact `packet` identity, read that Packet and every Evidence identity
in its `evidence_set`, then enqueue every Decision identity in that Packet's
`context.dependency_proofs`. Recurse until no unseen dependency Decision remains. Deduplicate each
kind by immutable exact identity so converging dependency paths load material once. At every step,
require the declared distinct identity count to equal the returned distinct row count; reject
extra, missing, duplicate, or identity-mismatched rows. Use exact-identity queries in short read
transactions, never an all-Evidence scan.

For every `core_snapshot` in the closure, derive the blob name from its canonical SHA-256 digest,
verify the file's digest and size, then inspect only the byte range needed for the Judgment.

Only decide after the frozen closure is complete and inspected. Re-read both heads and the current
Packet identity after constructing it; any change invalidates the closure. Missing material,
truncated output, unresolved artifact integrity, or any identity/count mismatch blocks the
Decision. Advisory reviews and worker votes never become formal Judgment automatically.
