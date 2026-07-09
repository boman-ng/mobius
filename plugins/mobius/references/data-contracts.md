# Mobius Data Contracts

Mobius stores authoritative project-private state as CSV. JSON is used for CLI and MCP transport.
CSV ledgers may contain compact JSON in `*_json` cells, but CSV remains the local source of truth.

## Storage Layout

```text
.mobius/
  .gitignore
  runs/
    codex-session-<codex_session_id>/
      run.csv
      yyyy-mm-dd-<objective-slug>/
        objective.md
        objective.csv
        work_items.csv
        criteria.csv
        routes.csv
        route_runs.csv
        budget.csv
        evidence.csv
        review_targets.csv
        review_judgments.csv
        review_runs.csv
        verdict.csv
```

## Canonical Model

- Objective: the explicit user-visible outcome.
- Work Item: one ordered unit of work inside an Objective.
- Criterion: an observable condition required by a Work Item.
- Route: a selected implementation path for one Work Item and its Criteria.
- Route Run: one execution instance of a Route.
- Timebox: the harness-internal time limit for a Route Run.
- Evidence: objective support for Criteria.
- Review Target: a one-shot frozen index of current ledgers.
- Review Judgment: a recorded reviewer decision and feedback action.
- Review Feedback: structured repair input, not failure budget.
- Verdict: the derived terminal adjudication.

Core relations:

```text
Objective decomposes_into Work Item
Work Item requires Criterion
Route addresses Criterion
Route Run executes Route
Route Run constrained_by Timebox
Evidence supports Criterion
Review Target freezes Work Item, Criteria, Evidence, Route Run
Review Judgment evaluates Review Target
Verdict adjudicates Objective
```

## Ledgers

`objective.csv`

```csv
schema,objective_id,run_id,objective_slug,status,created_at,updated_at,contract_path,contract_sha256_tail,locked,locked_at,locked_by
```

`work_items.csv`

```csv
schema,objective_id,revision,id,title,description,contract_status,required,depends_on_json,scope_json,work_json,gate_json,recovery_json,timebox_json,criteria_ids_json,locked,locked_at,locked_by,lock_hash
```

`criteria.csv`

```csv
schema,objective_id,id,work_item_id,requirement,observable_outcome,evidence_required_json,verifier_json,review_focus_json,required,status,evidence_ids_json,review_judgment_id,verified_at,locked,locked_at,locked_by,lock_hash
```

`routes.csv`

```csv
schema,objective_id,id,work_item_id,criterion_ids_json,rationale,status,created_at
```

`route_runs.csv`

```csv
schema,objective_id,id,route_id,work_item_id,status,started_at,finished_at,timebox_ms,failure_kind,review_judgment_id
```

`budget.csv`

```csv
schema,id,objective_id,work_item_id,criterion_id,route_id,route_run_id,review_target_id,review_run_id,tool_call_id,event_kind,clock_domain,metered,source,started_at,finished_at,duration_ms,consumed_ms,remaining_ms,failure_kind,created_at
```

`clock_domain` is one of `harness_internal`, `external_blocking`, `external_detached`, `mixed`, or
`unknown`. Only metered harness-internal time consumes Route Run timebox by default. Mixed tool
events must provide explicit `consumed_ms`.

Route Run expiry is derived only from rows linked by `route_run_id` where `metered=true` and
`clock_domain` is `harness_internal` or explicitly classified `mixed`. Other budget rows remain
accounting facts. When counted consumed time reaches `route_runs.csv.timebox_ms`, the Route Run
becomes `expired` with `failure_kind=timebox_expired`; Criterion, Objective, and Verdict state do
not change from that budget fact alone.

`codex-session-import` writes only events that carry a timestamp or paired timing bounds. Its
`source` cell includes the session file, line number, and precision marker such as
`precision=timestamp_only`, `precision=duration_ms`, or `precision=paired_timestamps_ms`. Imported
events use an explicit `clock_domain`/`mobius_clock_domain` field when present. Recognized model
generation events are classified as `harness_internal`; generic tool events without stable
classification are recorded as `unknown` and unmetered rather than silently consuming Timebox
budget.

`evidence.csv`

```csv
schema,id,objective_id,type,summary,supports_json,artifact_json,created_by,created_at
```

Evidence types are `change_set_scope`, `file_ref`, `command_result`, `test_result`, and
`human_assertion`.

`review_targets.csv`

```csv
schema,review_target_id,objective_id,objective_slug,review_mode,stateless,work_item_id,route_run_id,created_at,target_json,target_sha256
```

`review_mode` is `checkpoint_review` or `exit_review`. Each Review Target is one-shot input.

`review_judgments.csv`

```csv
schema,review_judgment_id,objective_id,review_target_id,review_mode,level,stateless,reviewers_json,result_json,feedback_action,raw_ref,raw_hash_tail,returned_at
```

`feedback_action` is one of `none`, `repair_route`, `add_evidence`, `select_alternate_route`,
`retry_review`, or `contract_change_required`.

`review_runs.csv`

```csv
schema,review_run_id,review_target_id,review_mode,status,started_at,finished_at,reviewer_summary_ref,failure_kind,retryable,diagnostic_ref
```

`verdict.csv`

```csv
schema,objective_id,overall,adjudicated_by,adjudicated_at,rule,derived_from_json,unverified_work_item_ids_json,unverified_criterion_ids_json,blocked_criterion_ids_json
```

## CLI Boundary

CLI commands return `mobius.command_result` JSON and, when relevant, a `mobius.loop` object.
Machine callers should follow `loop.next_argv` or `loop.next_actions`.

Primary commands:

```text
objective-start
contract-add-work-item
contract-validate
contract-lock
route-run-start
evidence-add
budget-add
codex-session-import
review-target-create
review-target-read
review-judgment-record
continue
loop-status
ledger-audit
explain
verdict
doctor
hook-health
```

Contract validation rejects retry-count budgeting keys such as `max_stage_attempts` and requires a
positive `route_run_timebox_ms` for every required Work Item.

## Review And Verdict

Checkpoint Review Judgment can pass Criteria for one Work Item. Exit Review Judgment must pass all
required Criteria before `verdict.csv` can become `accepted`. Non-pass checkpoint or exit Review
Judgment is ordinary feedback unless it is classified as `contract_change_required` or a true
terminal blocker. `retry_review` is recorded as a retryable Review Run infrastructure failure.
`repair_route`, `add_evidence`, and `select_alternate_route` route the loop to the matching next
action without consuming a Route failure budget.

Hooks protect direct writes to the authoritative ledgers and false terminal claims, but they do not
advance loop state, call reviewers, or decide the Verdict.
