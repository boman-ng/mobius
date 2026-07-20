# Formal Review read

Load only after a fresh read shows the selected Objective is `Reviewing`.

Freeze heads/root Packet; keep this closure ledger only in current context:

```text
Packet:   expected | read | kind/id verified
Decision: expected | read | kind/id verified
Evidence: expected | read | kind/id verified
Artifact: expected | integrity verified
Baseline: Evidence | bundle verified | current/superseded/unverifiable
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

Use literal non-writing operations to reject missing, symlink, non-regular, escaped, wrong-size, or
wrong-digest content. Hash the full-file SHA-256 as a stream; read one explicit
`[offset, offset + length)`; reject out-of-bounds/truncation; then repeat path/type/size/full-digest
verification after the range read. Use parent `shell_word` and no raw stored text. This is
observational only; never mutate the blob.

After integrity closure, apply `evidence-bundle.md`; classify every Bundle `current-applicable`,
`superseded`, or `unverifiable`. A mutable Criterion needs current applicable `supports`; address
current `contradicts`/`unknown` or do not accept.

Apply `risk-gate.md` to the exact frozen closure. Create one fresh required Judge task; any material
change makes it stale. Only a valid, current, complete Judge result with matched freeze/coverage
permits `accept`; absence or unusable advice blocks it. Unresolved Judge findings block `accept`;
main independently closes the Review.

Decide only after the ledger is complete. Re-read both heads, live `Reviewing` state, and the same
root Packet. Any drift, incomplete payload read, unresolved artifact integrity, truncation, or
count/identity mismatch blocks Decision. Advisory reviews and worker votes never become Judgment
automatically.
