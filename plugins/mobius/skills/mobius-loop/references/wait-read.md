# Formal Wait read

Load this recipe only after a fresh targeted read shows the selected Objective is `Waiting`.

Keep one ephemeral ledger: exact Wait/context input; chosen item/byte bounds; summary count/bytes;
returned identities; parse/admission result; final head/state recheck. Any unknown field is failure;
never persist or promote the ledger.

Choose finite literal `<max-evidence-items>` and `<max-payload-bytes>` ceilings from the current
command-output and Context budget. Reserve room for row labels, identities, and subsequent
reasoning; the payload ceiling must be smaller than the raw host-output allowance. These are
admission limits, not pagination. Replace the three placeholders below and run the statement in the
canonical read-only SQLite command from the parent Skill.

```sql
WITH current_wait AS MATERIALIZED (
  SELECT objective_id,
         json_extract(CAST(projection_bytes AS TEXT),
           '$.objective_state.navigating.navigation.waiting.wait_condition') AS wait_id
  FROM objective_projection
  WHERE objective_id = '<objective-id>'
), target_wait AS MATERIALIZED (
  SELECT c.objective_id, c.wait_id,
         json_extract(CAST(w.projection_bytes AS TEXT), '$.wait_condition.context') AS context
  FROM current_wait AS c
  JOIN object_projection AS w
    ON w.objective_id = c.objective_id AND w.object_kind = 'wait_condition'
  WHERE json_extract(CAST(w.projection_bytes AS TEXT), '$.wait_condition.id') = c.wait_id
), matching AS MATERIALIZED (
  SELECT e.object_id, e.projection_bytes
  FROM object_projection AS e
  JOIN target_wait AS w ON w.objective_id = e.objective_id
  WHERE e.object_kind = 'evidence'
    AND json_extract(CAST(e.projection_bytes AS TEXT), '$.evidence.purpose') = 'wait_resolution'
    AND json_extract(CAST(e.projection_bytes AS TEXT),
          '$.evidence.subject.wait_condition') = w.wait_id
    AND json_extract(CAST(e.projection_bytes AS TEXT), '$.evidence.context') = w.context
), stats AS MATERIALIZED (
  SELECT COUNT(*) AS matching_count,
         COALESCE(SUM(length(projection_bytes)), 0) AS payload_bytes
  FROM matching
), admission AS MATERIALIZED (
  SELECT matching_count, payload_bytes,
         matching_count <= <max-evidence-items>
           AND payload_bytes <= <max-payload-bytes> AS within_budget
  FROM stats
)
SELECT 'summary' AS row_kind, matching_count, payload_bytes, within_budget,
       NULL AS object_id, NULL AS payload
FROM admission
UNION ALL
SELECT 'evidence', NULL, NULL, NULL,
       object_id, CAST(projection_bytes AS TEXT)
FROM matching JOIN admission ON within_budget = 1
ORDER BY row_kind DESC, object_id;
```

Require `within_budget = 1` and an Evidence row count equal to `matching_count`. Budget denial
returns only the small summary. Budget denial, truncation, a missing summary, parse failure, or a
count mismatch keeps the Objective `Waiting`; do not submit `CheckWait`.

This query has no `LIMIT`: it returns the complete admitted set or none of its payloads. Do not use
it for ordinary exploration or any state other than the selected current Wait.
