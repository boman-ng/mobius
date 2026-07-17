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

For every `core_snapshot` in the closure, require exactly `digest` and `size_bytes`. Accept `digest`
only as `sha256:` followed by 64 lowercase hexadecimal characters and accept `size_bytes` only as a
non-negative integer. Strip only the `sha256:` prefix and construct this sole locator from the
already canonical project root:

```text
<canonical-project-root>/.mobius/artifacts/blobs/<digest-hex>
```

Before inspecting content, use the host's literal, non-writing filesystem operations to reject a
missing path, symlink, non-regular file, escaped canonical path, size mismatch, or full-file SHA-256
mismatch. Hash the file as a stream; do not place the full blob in Context. After that verification,
read only one explicitly chosen `[offset, offset + length)` range needed for the Judgment, require
the range to be within `size_bytes`, and reject short or truncated output. Repeat the regular-file,
canonical-path, full digest, and size verification after the range read; any change or unverifiable
check invalidates the material. Shell paths, when a shell adapter is used, must be encoded with the
parent Skill's `shell_word` rule and must never contain raw stored text. This procedure is
observational only; no Agent operation may create, rename, rewrite, chmod, or delete the blob.

Only decide after the frozen closure is complete and inspected. Re-read both heads and the current
Packet identity after constructing it; any change invalidates the closure. Missing material,
truncated output, unresolved artifact integrity, or any identity/count mismatch blocks the
Decision. Advisory reviews and worker votes never become formal Judgment automatically.
