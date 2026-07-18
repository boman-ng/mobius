# Formal Review read

Load this recipe only after a fresh targeted read shows the selected Objective is `Reviewing`.

Freeze both heads and the root Packet identity. Keep this closure ledger only in current context:

```text
Packet:   expected | read | kind/id verified
Decision: expected | read | kind/id verified
Evidence: expected | read | kind/id verified
Artifact: expected | integrity verified
Final heads/root Packet: pending | passed
```

Start with the exact root `review_packet`. Enqueue every distinct Evidence in its `evidence_set`
and every Decision in `context.dependency_proofs`. For each unseen dependency Decision, read its
exact `review_decision`, enqueue its exact Packet, then enqueue that Packet's Evidence and
dependency Decisions. Recurse until no unseen identity remains; deduplicate each kind by immutable
identity.

Mark an identity read only after an exact query returns one row, returns `projection_bytes`, has
the expected kind, and its embedded identity matches. `COUNT(*)`, kind inventory, a prior-session
read, a worker summary, or an earlier Stage review never marks an identity read. Require each
declared distinct count to equal its read and verified distinct counts. Any extra, missing,
duplicate, or mismatched row invalidates the closure.

For each `core_snapshot`, require exactly `digest` and `size_bytes`. Accept a digest only as
`sha256:` plus 64 lowercase hexadecimal characters and size only as a non-negative integer. Strip
the prefix and construct the sole locator:

```text
<canonical-project-root>/.mobius/artifacts/blobs/<digest-hex>
```

Before reading content, use literal non-writing filesystem operations to reject a missing path,
symlink, non-regular file, escaped canonical path, size mismatch, or full-file SHA-256 mismatch.
Hash as a stream. Read only one explicit `[offset, offset + length)` range needed for Judgment;
reject an out-of-bounds or truncated range. Repeat the canonical-path, regular-file, full digest,
and size verification after the range read. Shell paths must use the parent Skill's `shell_word`
rule and contain no raw stored text. The procedure is observational only: never create, rename,
rewrite, chmod, or delete the blob.

Decide only after the ledger is complete. Re-read both heads, live `Reviewing` state, and the same
root Packet. Any drift, incomplete payload read, unresolved artifact integrity, truncation, or
count/identity mismatch blocks Decision. Advisory reviews and worker votes never become Judgment
automatically.
