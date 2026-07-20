use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value, json};

fn repository_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(3)
        .expect("runtime must remain under plugins/mobius")
        .to_path_buf()
}

fn read(relative: &str) -> String {
    let path = repository_root().join(relative);
    fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()))
}

fn fixture() -> Value {
    serde_json::from_str(&read(
        "plugins/mobius/runtime/tests/fixtures/session-audit-d0-v1.json",
    ))
    .expect("synthetic D0 fixture must be valid JSON")
}

fn object<'a>(value: &'a Value, label: &str) -> &'a Map<String, Value> {
    value
        .as_object()
        .unwrap_or_else(|| panic!("{label} must be an object"))
}

fn array<'a>(value: &'a Value, label: &str) -> &'a [Value] {
    value
        .as_array()
        .unwrap_or_else(|| panic!("{label} must be an array"))
}

fn string<'a>(value: &'a Value, label: &str) -> &'a str {
    value
        .as_str()
        .unwrap_or_else(|| panic!("{label} must be a string"))
}

fn number(value: &Value, label: &str) -> u64 {
    value
        .as_u64()
        .unwrap_or_else(|| panic!("{label} must be a non-negative integer"))
}

fn assert_exact_keys(value: &Value, expected: &[&str], label: &str) {
    let actual = object(value, label)
        .keys()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    let expected = expected.iter().copied().collect::<BTreeSet<_>>();
    assert_eq!(actual, expected, "{label} must be closed");
}

fn is_sha256(value: &Value) -> bool {
    value.as_str().is_some_and(|value| {
        value.strip_prefix("sha256:").is_some_and(|hex| {
            hex.len() == 64
                && hex
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        })
    })
}

fn is_digest_hint(value: &Value) -> bool {
    value.as_str().is_some_and(|value| {
        value.strip_prefix("sha256:…").is_some_and(|suffix| {
            suffix.len() == 7
                && suffix
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        })
    })
}

fn contains_full_sha256(value: &Value) -> bool {
    match value {
        Value::Array(values) => values.iter().any(contains_full_sha256),
        Value::Object(values) => values.values().any(contains_full_sha256),
        _ => is_sha256(value),
    }
}

fn contains_prose(haystack: &str, needle: &str) -> bool {
    let normalize = |text: &str| text.split_whitespace().collect::<Vec<_>>().join(" ");
    normalize(haystack).contains(&normalize(needle))
}

fn contains_uuid_like(text: &str) -> bool {
    text.as_bytes().windows(36).any(|candidate| {
        candidate.iter().enumerate().all(|(index, byte)| {
            if [8, 13, 18, 23].contains(&index) {
                *byte == b'-'
            } else {
                byte.is_ascii_hexdigit()
            }
        })
    })
}

fn runtime_record_id(observation: &Value) -> String {
    format!(
        "runtime-{:02}",
        number(&observation["order"], "Runtime order")
    )
}

fn trail_record_id(identity: &Value) -> String {
    format!("trail-{:02}", number(&identity["order"], "Trail order"))
}

#[derive(Default)]
struct DigestReferences {
    local_ids: BTreeMap<String, String>,
    display_owners: BTreeMap<String, String>,
}

impl DigestReferences {
    fn render(&mut self, digest: &Value, preferred_local_id: String) -> Value {
        assert!(
            is_sha256(digest),
            "source correlation digest must be full SHA-256"
        );
        let digest = string(digest, "source digest");
        let local_id = self
            .local_ids
            .entry(digest.to_owned())
            .or_insert(preferred_local_id)
            .clone();
        let suffix = &digest[digest.len() - 7..];
        let hint = format!("sha256:…{suffix}");
        let display = match self.display_owners.get(&hint) {
            Some(owner) if owner != digest => Value::Null,
            Some(_) => Value::String(hint),
            None => {
                self.display_owners.insert(hint.clone(), digest.to_owned());
                Value::String(hint)
            }
        };
        json!({
            "local_id": local_id,
            "display": display,
        })
    }

    fn render_text(&mut self, digest: &str, preferred_local_id: String) -> Value {
        self.render(&Value::String(digest.to_owned()), preferred_local_id)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct CorrelationKey {
    request_id: String,
    objective_id: String,
    transition: String,
    object_kind: String,
    object_id: String,
    event_digest: String,
}

impl CorrelationKey {
    fn typed_identity(&self) -> Value {
        json!({
            "objective_id": self.objective_id,
            "transition": self.transition,
            "object_kind": self.object_kind,
            "object_id": self.object_id,
        })
    }
}

#[derive(Clone, Debug)]
struct IdentityRecord {
    record_id: String,
    key: CorrelationKey,
}

fn correlation_key(record: &Value, digest_field: &str, label: &str) -> CorrelationKey {
    assert_exact_keys(
        &record["typed_identity"],
        &["objective_id", "transition", "object_kind", "object_id"],
        &format!("{label} typed identity"),
    );
    let typed = &record["typed_identity"];
    assert!(
        is_sha256(&record[digest_field]),
        "{label} digest must be full"
    );
    CorrelationKey {
        request_id: string(&record["request_id"], &format!("{label} request_id")).to_owned(),
        objective_id: string(&typed["objective_id"], &format!("{label} Objective")).to_owned(),
        transition: string(&typed["transition"], &format!("{label} transition")).to_owned(),
        object_kind: string(&typed["object_kind"], &format!("{label} object kind")).to_owned(),
        object_id: string(&typed["object_id"], &format!("{label} object id")).to_owned(),
        event_digest: string(&record[digest_field], &format!("{label} digest")).to_owned(),
    }
}

fn runtime_identities(fixture: &Value) -> Vec<IdentityRecord> {
    array(&fixture["runtime_observations"], "Runtime observations")
        .iter()
        .filter(|record| record.get("typed_identity").is_some())
        .map(|record| IdentityRecord {
            record_id: runtime_record_id(record),
            key: correlation_key(record, "receipt_event_digest", "Runtime correlation"),
        })
        .collect()
}

fn trail_identities(fixture: &Value) -> Vec<IdentityRecord> {
    array(&fixture["trail"]["identities"], "Trail identities")
        .iter()
        .map(|record| IdentityRecord {
            record_id: trail_record_id(record),
            key: correlation_key(record, "event_digest", "Trail correlation"),
        })
        .collect()
}

fn result_classification(result: &str) -> &'static str {
    match result {
        "accepted" => "accepted",
        "idempotent_noop" => "no_op",
        "stale_head" => "rejected",
        "invalid" | "invalid_tool_input" => "invalid",
        _ => "unknown",
    }
}

fn derive_mutation_attempts(fixture: &Value, digests: &mut DigestReferences) -> Value {
    let observations = array(&fixture["runtime_observations"], "Runtime observations");
    let mut items = Vec::new();
    for observation in observations {
        if observation.get("request_id").is_none()
            || observation.get("payload_digest").is_none()
            || observation.get("heads").is_none()
        {
            continue;
        }
        let record_id = runtime_record_id(observation);
        let heads = array(&observation["heads"], "mutation heads");
        assert_eq!(
            heads.len(),
            2,
            "mutation heads must have project and Objective values"
        );
        let result = string(&observation["result"], "Runtime result");
        items.push(json!({
            "runtime_record_id": record_id,
            "order": observation["order"],
            "kind": observation["kind"],
            "tool": observation["tool"],
            "request_id": observation["request_id"],
            "payload_digest": digests.render(
                &observation["payload_digest"],
                format!("digest-payload-{record_id}"),
            ),
            "heads": {
                "project_seq": heads[0],
                "objective_seq": heads[1],
            },
            "result": {
                "classification": result_classification(result),
                "detail": result,
            },
            "fault_class": observation.get("fault_class").cloned().unwrap_or(Value::Null),
        }));
    }

    let stale_head_rejections = observations
        .iter()
        .filter(|item| item["result"] == "stale_head")
        .count();
    let mut seen_faults = BTreeSet::new();
    let malformed_fault_classes = observations
        .iter()
        .filter(|item| {
            matches!(
                item["result"].as_str(),
                Some("invalid" | "invalid_tool_input")
            )
        })
        .filter_map(|item| item["fault_class"].as_str())
        .filter(|fault| seen_faults.insert((*fault).to_owned()))
        .collect::<Vec<_>>();
    let duplicate_init_noops = observations
        .iter()
        .filter(|item| {
            item["result"] == "idempotent_noop" && item["fault_class"] == "duplicate_project_init"
        })
        .count();

    json!({
        "bounds": {
            "max_records": fixture["input"]["bounds"]["max_records"],
            "observed_records": observations.len(),
            "emitted_attempts": items.len(),
            "truncated": false,
        },
        "counts": {
            "stale_head_rejections": stale_head_rejections,
            "malformed_fault_classes": malformed_fault_classes,
            "duplicate_init_noops": duplicate_init_noops,
        },
        "items": items,
    })
}

fn correlation_link(
    correlation_id: String,
    runtime_record_id: Option<&str>,
    trail_record_id: Option<&str>,
    key: &CorrelationKey,
    reason: Option<&str>,
    digest_local_id: String,
    digests: &mut DigestReferences,
) -> Value {
    let classification = if reason.is_some() {
        "unknown"
    } else {
        "matched"
    };
    json!({
        "correlation_id": correlation_id,
        "classification": classification,
        "runtime_record_id": runtime_record_id,
        "trail_record_id": trail_record_id,
        "request_id": key.request_id,
        "typed_identity": key.typed_identity(),
        "event_digest": digests.render_text(&key.event_digest, digest_local_id),
        "reason": reason,
    })
}

fn derive_correlations(fixture: &Value, digests: &mut DigestReferences) -> Value {
    let runtime = runtime_identities(fixture);
    let trail = trail_identities(fixture);
    let runtime_counts = runtime.iter().fold(BTreeMap::new(), |mut counts, record| {
        *counts.entry(record.key.clone()).or_insert(0_usize) += 1;
        counts
    });
    let trail_counts = trail.iter().fold(BTreeMap::new(), |mut counts, record| {
        *counts.entry(record.key.clone()).or_insert(0_usize) += 1;
        counts
    });

    let mut matched = Vec::new();
    let mut unknown = Vec::new();
    let mut matched_trail_records = BTreeSet::new();
    let mut matched_index = 0_usize;
    let mut unknown_runtime_index = 0_usize;

    for runtime_record in &runtime {
        let is_exact_unique_match = runtime_counts[&runtime_record.key] == 1
            && trail_counts.get(&runtime_record.key) == Some(&1);
        if is_exact_unique_match {
            matched_index += 1;
            let trail_record = trail
                .iter()
                .find(|candidate| candidate.key == runtime_record.key)
                .expect("exact Trail match exists");
            matched_trail_records.insert(trail_record.record_id.clone());
            matched.push(correlation_link(
                format!("correlation-matched-{matched_index:02}"),
                Some(&runtime_record.record_id),
                Some(&trail_record.record_id),
                &runtime_record.key,
                None,
                format!("digest-event-{}", runtime_record.record_id),
                digests,
            ));
        } else {
            unknown_runtime_index += 1;
            unknown.push(correlation_link(
                format!("correlation-unknown-runtime-{unknown_runtime_index:02}"),
                Some(&runtime_record.record_id),
                None,
                &runtime_record.key,
                Some("no_exact_trail_identity"),
                format!("digest-event-{}", runtime_record.record_id),
                digests,
            ));
        }
    }

    let mut unknown_trail_index = 0_usize;
    for trail_record in &trail {
        if matched_trail_records.contains(&trail_record.record_id) {
            continue;
        }
        unknown_trail_index += 1;
        unknown.push(correlation_link(
            format!("correlation-unknown-trail-{unknown_trail_index:02}"),
            None,
            Some(&trail_record.record_id),
            &trail_record.key,
            Some("no_exact_runtime_identity"),
            format!("digest-event-{}", trail_record.record_id),
            digests,
        ));
    }

    json!({"matched": matched, "unknown": unknown})
}

fn derive_trail_summary(fixture: &Value, digests: &mut DigestReferences) -> Value {
    let trail = &fixture["trail"];
    let cycles = array(&trail["stage_cycles"], "Trail stage cycles");
    let accepted_cycles = cycles
        .iter()
        .map(|cycle| {
            json!({
                "stage": cycle["stage"],
                "transitions": cycle["transitions"],
                "decision": cycle["decision"],
            })
        })
        .collect::<Vec<_>>();
    let evidence = cycles
        .iter()
        .map(|cycle| {
            let stage = string(&cycle["stage"], "evidence stage");
            json!({
                "stage": stage,
                "baseline_digest": digests.render(
                    &cycle["evidence_baseline"],
                    format!("digest-evidence-{stage}"),
                ),
                "snapshot_identity": cycle["snapshot_identity"],
                "claims": cycle["claims"],
                "freshness": cycle["freshness"],
            })
        })
        .collect::<Vec<_>>();

    json!({
        "authority": fixture["authority"]["business_facts"],
        "accepted_order": {
            "prefix": trail["prefix"],
            "stage_cycles": accepted_cycles,
        },
        "evidence": evidence,
        "projected_final_state": trail["projected_final_state"],
    })
}

fn role_timing(subagents: &[Value], role: &str) -> Value {
    let matching = subagents
        .iter()
        .filter(|agent| agent["role"] == role)
        .collect::<Vec<_>>();
    if matching.is_empty() {
        return json!({
            "count": 0,
            "status": "absent",
            "relative_to_attempt": "absent",
            "relative_to_seal": "absent",
            "relative_to_review": "absent",
        });
    }

    let timing = &matching[0]["timing"];
    assert_exact_keys(
        timing,
        &[
            "relative_to_attempt",
            "relative_to_seal",
            "relative_to_review",
        ],
        &format!("{role} timing source"),
    );
    for agent in &matching[1..] {
        assert_eq!(
            agent["timing"], *timing,
            "{role} timing must aggregate exactly"
        );
    }
    json!({
        "count": matching.len(),
        "status": "observed",
        "relative_to_attempt": timing["relative_to_attempt"],
        "relative_to_seal": timing["relative_to_seal"],
        "relative_to_review": timing["relative_to_review"],
    })
}

fn derive_lifecycle_summary(fixture: &Value) -> Value {
    let cycles = array(&fixture["trail"]["stage_cycles"], "Trail stage cycles");
    let subagents = array(&fixture["subagents"], "Subagents");
    for cycle in cycles {
        let transitions = array(&cycle["transitions"], "Stage transitions");
        for required in ["start_attempt", "seal_attempt", "decision"] {
            assert!(
                transitions.iter().any(|transition| transition == required),
                "accepted Stage cycle omitted {required}"
            );
        }
        assert_eq!(cycle["decision"], "accept");
    }
    assert_eq!(fixture["judge"]["status"], "absent");

    json!({
        "accepted_stage_cycles": cycles.len(),
        "role_timing": {
            "driver": role_timing(subagents, "driver"),
            "verifier": role_timing(subagents, "verifier"),
            "judge": role_timing(subagents, "judge"),
        },
    })
}

fn derive_subagent_summary(fixture: &Value, digests: &mut DigestReferences) -> Value {
    let cycles = array(&fixture["trail"]["stage_cycles"], "Trail stage cycles");
    let subagents = array(&fixture["subagents"], "Subagents");
    let mut verifiers_by_stage = cycles
        .iter()
        .map(|cycle| (string(&cycle["stage"], "Stage id").to_owned(), 0_u64))
        .collect::<BTreeMap<_, _>>();
    let mut drivers = 0_u64;
    let mut stage_judges = 0_u64;
    let mut tasks = Vec::new();

    for agent in subagents {
        let task_id = string(&agent["opaque_task"], "opaque task");
        match string(&agent["role"], "Subagent role") {
            "verifier" => {
                *verifiers_by_stage
                    .get_mut(string(&agent["stage"], "Verifier stage"))
                    .expect("Verifier stage is present in the Trail") += 1;
            }
            "driver" => drivers += 1,
            "judge" => stage_judges += 1,
            role => panic!("unexpected role {role}"),
        }
        tasks.push(json!({
            "task_id": task_id,
            "role": agent["role"],
            "stage": agent["stage"],
            "task_baseline": digests.render(
                &agent["task_baseline"],
                format!("digest-baseline-{task_id}"),
            ),
            "spawn_status": agent["spawn_status"],
            "final_status": agent["final_status"],
            "close_status": agent["close_status"],
            "effects": agent["effects"],
            "consumed_before": agent["consumed_before"],
        }));
    }

    json!({
        "counts": {
            "verifiers_by_stage": verifiers_by_stage,
            "drivers": drivers,
            "stage_judges": stage_judges,
        },
        "tasks": tasks,
    })
}

fn derive_agent_path_conformance(fixture: &Value) -> Value {
    let cycles = array(&fixture["trail"]["stage_cycles"], "Trail stage cycles");
    let observed_judge_stages = array(&fixture["subagents"], "Subagents")
        .iter()
        .filter(|agent| agent["role"] == "judge")
        .map(|agent| string(&agent["stage"], "Judge stage").to_owned())
        .collect::<BTreeSet<_>>();
    let missing_judge_stages = cycles
        .iter()
        .map(|cycle| string(&cycle["stage"], "Stage id"))
        .filter(|stage| !observed_judge_stages.contains(*stage))
        .map(str::to_owned)
        .collect::<Vec<_>>();

    json!({
        "status": if missing_judge_stages.is_empty() {
            "conformant"
        } else {
            "nonconformant"
        },
        "rule": "one_fresh_complete_judge_before_each_stage_review_decision",
        "required_stage_reviews": cycles.len(),
        "observed_stage_judges": observed_judge_stages.len(),
        "missing_judge_stages": missing_judge_stages,
    })
}

fn derive_deviations(
    fixture: &Value,
    correlations: &Value,
    agent_path_conformance: &Value,
) -> Vec<Value> {
    let observations = array(&fixture["runtime_observations"], "Runtime observations");
    let mut deviations = Vec::new();
    for observation in observations {
        let code = observation["fault_class"]
            .as_str()
            .or_else(|| (observation["result"] == "stale_head").then_some("stale_head"));
        if let Some(code) = code {
            deviations.push(json!({
                "subject_ref": runtime_record_id(observation),
                "code": code,
                "classification": "runtime_observation",
            }));
        }
    }
    for link in array(&correlations["unknown"], "unknown correlations") {
        if let Some(runtime_record_id) = link["runtime_record_id"].as_str() {
            deviations.push(json!({
                "subject_ref": runtime_record_id,
                "code": "unresolved_runtime_trail_identity",
                "classification": "unknown",
            }));
        }
    }
    for stage in array(
        &agent_path_conformance["missing_judge_stages"],
        "missing Judge stages",
    ) {
        deviations.push(json!({
            "subject_ref": format!("stage:{}", string(stage, "missing Judge Stage")),
            "code": "missing_required_stage_judge",
            "classification": "agent_path_nonconformance",
        }));
    }
    deviations
}

fn derive_expected_output(fixture: &Value) -> Value {
    let mut digests = DigestReferences::default();
    let session_label_digest = digests.render(
        &fixture["session_label_digest"],
        "digest-session-label".to_owned(),
    );
    let mutation_attempts = derive_mutation_attempts(fixture, &mut digests);
    let correlations = derive_correlations(fixture, &mut digests);
    let trail_summary = derive_trail_summary(fixture, &mut digests);
    let lifecycle_summary = derive_lifecycle_summary(fixture);
    let subagent_summary = derive_subagent_summary(fixture, &mut digests);
    let agent_path_conformance = derive_agent_path_conformance(fixture);
    let deviations = derive_deviations(fixture, &correlations, &agent_path_conformance);
    let unknown = array(&correlations["unknown"], "unknown correlations");
    let unresolved_links = unknown
        .iter()
        .map(|link| link["correlation_id"].clone())
        .collect::<Vec<_>>();
    let unknown_runtime = unknown
        .iter()
        .filter(|link| !link["runtime_record_id"].is_null())
        .count();
    let unknown_trail = unknown
        .iter()
        .filter(|link| !link["trail_record_id"].is_null())
        .count();
    let missing_judge_count = agent_path_conformance["missing_judge_stages"]
        .as_array()
        .expect("missing Judge stages are an array")
        .len();
    let judge = &fixture["judge"];
    let completion = &fixture["completion"];

    json!({
        "schema": "mobius.session-audit-output.v1",
        "session_label_digest": session_label_digest,
        "binding": {
            "project_id": fixture["source_binding"]["project_id"],
            "objective_id": fixture["source_binding"]["objective_id"],
        },
        "mutation_attempts": mutation_attempts,
        "correlations": correlations,
        "trail_summary": trail_summary,
        "lifecycle_summary": lifecycle_summary,
        "subagent_summary": subagent_summary,
        "judge_summary": {
            "status": judge["status"],
            "reason": judge["reason"],
        },
        "agent_path_conformance": agent_path_conformance,
        "completion_summary": {
            "authority": fixture["authority"]["business_facts"],
            "objective_state": fixture["trail"]["projected_final_state"],
            "audit": completion["audit"],
            "marker": completion["marker"],
        },
        "deviations": deviations,
        "unresolved_links": unresolved_links,
        "residual_risks": [
            {
                "code": "unresolved_runtime_observation",
                "status": "open",
                "count": unknown_runtime,
            },
            {
                "code": "unresolved_trail_identity",
                "status": "open",
                "count": unknown_trail,
            },
            {
                "code": "missing_required_stage_judge",
                "status": "open",
                "count": missing_judge_count,
            },
        ],
    })
}

fn assert_digest_reference(value: &Value, label: &str) {
    assert_exact_keys(value, &["local_id", "display"], label);
    assert!(
        string(&value["local_id"], &format!("{label} local id")).starts_with("digest-"),
        "{label} must use a task-local digest id"
    );
    assert!(
        value["display"].is_null() || is_digest_hint(&value["display"]),
        "{label} must use a seven-hex display hint or local-id-only collision fallback"
    );
}

fn validate_expected_output_schema(output: &Value) {
    assert_exact_keys(
        output,
        &[
            "schema",
            "session_label_digest",
            "binding",
            "mutation_attempts",
            "correlations",
            "trail_summary",
            "lifecycle_summary",
            "subagent_summary",
            "judge_summary",
            "agent_path_conformance",
            "completion_summary",
            "deviations",
            "unresolved_links",
            "residual_risks",
        ],
        "expected output",
    );
    assert_exact_keys(
        &output["binding"],
        &["project_id", "objective_id"],
        "output binding",
    );
    assert_digest_reference(&output["session_label_digest"], "session label digest");

    let attempts = &output["mutation_attempts"];
    assert_exact_keys(
        attempts,
        &["bounds", "counts", "items"],
        "mutation attempts",
    );
    assert_exact_keys(
        &attempts["bounds"],
        &[
            "max_records",
            "observed_records",
            "emitted_attempts",
            "truncated",
        ],
        "mutation bounds",
    );
    assert_exact_keys(
        &attempts["counts"],
        &[
            "stale_head_rejections",
            "malformed_fault_classes",
            "duplicate_init_noops",
        ],
        "mutation counts",
    );
    for item in array(&attempts["items"], "mutation items") {
        assert_exact_keys(
            item,
            &[
                "runtime_record_id",
                "order",
                "kind",
                "tool",
                "request_id",
                "payload_digest",
                "heads",
                "result",
                "fault_class",
            ],
            "mutation item",
        );
        assert_digest_reference(&item["payload_digest"], "mutation payload digest");
        assert_exact_keys(
            &item["heads"],
            &["project_seq", "objective_seq"],
            "mutation heads",
        );
        assert_exact_keys(
            &item["result"],
            &["classification", "detail"],
            "mutation result",
        );
    }

    let correlations = &output["correlations"];
    assert_exact_keys(correlations, &["matched", "unknown"], "correlations");
    for link in array(&correlations["matched"], "matched correlations")
        .iter()
        .chain(array(&correlations["unknown"], "unknown correlations"))
    {
        assert_exact_keys(
            link,
            &[
                "correlation_id",
                "classification",
                "runtime_record_id",
                "trail_record_id",
                "request_id",
                "typed_identity",
                "event_digest",
                "reason",
            ],
            "correlation link",
        );
        assert_exact_keys(
            &link["typed_identity"],
            &["objective_id", "transition", "object_kind", "object_id"],
            "correlation typed identity",
        );
        assert_digest_reference(&link["event_digest"], "correlation event digest");
    }

    let trail = &output["trail_summary"];
    assert_exact_keys(
        trail,
        &[
            "authority",
            "accepted_order",
            "evidence",
            "projected_final_state",
        ],
        "Trail summary",
    );
    assert_exact_keys(
        &trail["accepted_order"],
        &["prefix", "stage_cycles"],
        "accepted order",
    );
    for cycle in array(
        &trail["accepted_order"]["stage_cycles"],
        "accepted Stage cycles",
    ) {
        assert_exact_keys(
            cycle,
            &["stage", "transitions", "decision"],
            "accepted Stage cycle",
        );
    }
    for evidence in array(&trail["evidence"], "evidence summaries") {
        assert_exact_keys(
            evidence,
            &[
                "stage",
                "baseline_digest",
                "snapshot_identity",
                "claims",
                "freshness",
            ],
            "evidence summary",
        );
        assert_digest_reference(&evidence["baseline_digest"], "evidence baseline digest");
    }

    let lifecycle = &output["lifecycle_summary"];
    assert_exact_keys(
        lifecycle,
        &["accepted_stage_cycles", "role_timing"],
        "lifecycle summary",
    );
    assert_exact_keys(
        &lifecycle["role_timing"],
        &["driver", "verifier", "judge"],
        "role timing",
    );
    for role in ["driver", "verifier", "judge"] {
        assert_exact_keys(
            &lifecycle["role_timing"][role],
            &[
                "count",
                "status",
                "relative_to_attempt",
                "relative_to_seal",
                "relative_to_review",
            ],
            &format!("{role} timing"),
        );
    }

    let subagents = &output["subagent_summary"];
    assert_exact_keys(subagents, &["counts", "tasks"], "Subagent summary");
    assert_exact_keys(
        &subagents["counts"],
        &["verifiers_by_stage", "drivers", "stage_judges"],
        "Subagent counts",
    );
    assert_exact_keys(
        &subagents["counts"]["verifiers_by_stage"],
        &["S1", "S2", "S3", "S4", "S5"],
        "Verifier counts by Stage",
    );
    for task in array(&subagents["tasks"], "Subagent tasks") {
        assert_exact_keys(
            task,
            &[
                "task_id",
                "role",
                "stage",
                "task_baseline",
                "spawn_status",
                "final_status",
                "close_status",
                "effects",
                "consumed_before",
            ],
            "Subagent task summary",
        );
        assert_digest_reference(&task["task_baseline"], "Subagent task baseline");
    }

    assert_exact_keys(
        &output["judge_summary"],
        &["status", "reason"],
        "Judge summary",
    );
    assert_exact_keys(
        &output["agent_path_conformance"],
        &[
            "status",
            "rule",
            "required_stage_reviews",
            "observed_stage_judges",
            "missing_judge_stages",
        ],
        "Agent path conformance",
    );
    assert_exact_keys(
        &output["completion_summary"],
        &["authority", "objective_state", "audit", "marker"],
        "completion summary",
    );
    for deviation in array(&output["deviations"], "deviations") {
        assert_exact_keys(
            deviation,
            &["subject_ref", "code", "classification"],
            "deviation",
        );
    }
    for risk in array(&output["residual_risks"], "residual risks") {
        assert_exact_keys(risk, &["code", "status", "count"], "residual risk");
    }
}

#[test]
fn d0_contract_keeps_runtime_observation_separate_from_trail_authority() {
    let contract = read("docs/session-audit-contract.md");
    for required in [
        "mobius.session-audit-input.v1",
        "mobius.session-audit-output.v1",
        "mobius.session-audit-redaction.v1",
        "Trail is the sole authority",
        "execution observations",
        "It never upgrades those observations into Mobius facts",
        "An ambiguous or missing match is `unknown`",
        "request id, typed Objective/transition/object identity, and receipt/event digest",
        "A missing Judge task or result is `absent`",
        "Agent-path nonconformance",
        "does not make the Core audit unhealthy",
        "five missing required Stage Judge rituals",
        "one bounded streaming pass",
        "overall audit `degraded`",
        "Do not return a complete-looking partial timeline",
        "Output goes to stdout by default",
        "Do not write `.mobius`",
        "D1 is `not_evaluated`",
    ] {
        assert!(
            contains_prose(&contract, required),
            "D0 contract omitted {required}"
        );
    }
    for forbidden in [
        "persistent transcript index",
        "automatic success fallback",
        "session report becomes Evidence",
    ] {
        assert!(!contract.contains(forbidden));
    }
}

#[test]
fn synthetic_d0_fixture_has_closed_input_binding_and_positive_bounds() {
    let fixture = fixture();
    assert_exact_keys(
        &fixture,
        &[
            "schema",
            "authority",
            "input",
            "source_binding",
            "session_label_digest",
            "runtime_observations",
            "trail",
            "subagents",
            "judge",
            "completion",
            "expected_output",
        ],
        "D0 fixture",
    );
    assert_eq!(fixture["schema"], "mobius.session-audit-fixture.v1");
    assert_exact_keys(
        &fixture["authority"],
        &["business_facts", "runtime_scope"],
        "authority",
    );
    assert_eq!(fixture["authority"]["business_facts"], "trail");
    assert_eq!(
        fixture["authority"]["runtime_scope"],
        "execution_observation_only"
    );

    let input = &fixture["input"];
    assert_exact_keys(
        input,
        &[
            "adapter_id",
            "bounds",
            "objective_id",
            "redaction_profile",
            "schema",
            "session_locator",
            "source_schema",
        ],
        "D0 input",
    );
    assert_eq!(input["schema"], "mobius.session-audit-input.v1");
    assert_eq!(
        input["redaction_profile"],
        "mobius.session-audit-redaction.v1"
    );
    assert_exact_keys(
        &input["source_schema"],
        &["version_or_fingerprint"],
        "source schema",
    );
    assert!(is_sha256(&input["source_schema"]["version_or_fingerprint"]));
    assert!(
        string(&input["session_locator"], "session locator").starts_with("synthetic://authorized/"),
        "checked-in locator must be explicitly synthetic and authorized"
    );
    assert!(
        string(&input["adapter_id"], "adapter id").starts_with("synthetic."),
        "checked-in adapter must be synthetic"
    );

    assert_exact_keys(
        &input["bounds"],
        &[
            "max_agents",
            "max_input_bytes",
            "max_records",
            "max_result_bytes",
        ],
        "D0 bounds",
    );
    for bound in [
        "max_input_bytes",
        "max_records",
        "max_agents",
        "max_result_bytes",
    ] {
        assert!(
            number(&input["bounds"][bound], bound) > 0,
            "bound {bound} must be positive"
        );
    }

    assert_exact_keys(
        &fixture["source_binding"],
        &["project_id", "objective_id"],
        "source binding",
    );
    assert!(
        string(&fixture["source_binding"]["project_id"], "source project")
            .starts_with("project-synthetic-")
    );
    assert!(
        string(
            &fixture["source_binding"]["objective_id"],
            "source Objective"
        )
        .starts_with("objective-synthetic-")
    );
    assert_eq!(
        fixture["source_binding"]["objective_id"],
        input["objective_id"]
    );
    assert!(is_sha256(&fixture["session_label_digest"]));

    let raw = read("plugins/mobius/runtime/tests/fixtures/session-audit-d0-v1.json");
    assert!(raw.len() as u64 <= number(&input["bounds"]["max_input_bytes"], "max input bytes"));
    assert!(
        array(&fixture["runtime_observations"], "Runtime observations").len() as u64
            <= number(&input["bounds"]["max_records"], "max records")
    );
    assert!(
        array(&fixture["subagents"], "Subagents").len() as u64
            <= number(&input["bounds"]["max_agents"], "max agents")
    );
    assert!(
        serde_json::to_vec(&fixture["expected_output"])
            .unwrap()
            .len() as u64
            <= number(&input["bounds"]["max_result_bytes"], "max result bytes")
    );
    assert_eq!(
        object(&fixture, "fixture")
            .keys()
            .filter(|key| key.starts_with("expected"))
            .count(),
        1,
        "fixture must contain one expected_output contract"
    );
}

#[test]
fn expected_output_is_derived_from_runtime_and_trail_sources() {
    let fixture = fixture();
    let derived = derive_expected_output(&fixture);
    assert_eq!(derived, fixture["expected_output"]);
    validate_expected_output_schema(&fixture["expected_output"]);
}

#[test]
fn exact_runtime_trail_identities_yield_one_match_and_explicit_unknowns() {
    let fixture = fixture();
    let runtime = runtime_identities(&fixture);
    let trail = trail_identities(&fixture);
    assert_eq!(runtime.len(), 2);
    assert_eq!(trail.len(), 2);

    let matched_runtime = runtime
        .iter()
        .find(|record| record.record_id == "runtime-07")
        .expect("matched Runtime identity exists");
    let matched_trail = trail
        .iter()
        .find(|record| record.record_id == "trail-30")
        .expect("matched Trail identity exists");
    assert_eq!(matched_runtime.key, matched_trail.key);

    let unknown_runtime = runtime
        .iter()
        .find(|record| record.record_id == "runtime-08")
        .expect("unmatched Runtime identity exists");
    let unknown_trail = trail
        .iter()
        .find(|record| record.record_id == "trail-32")
        .expect("unmatched Trail identity exists");
    assert_eq!(unknown_runtime.key.request_id, unknown_trail.key.request_id);
    assert_eq!(
        unknown_runtime.key.objective_id,
        unknown_trail.key.objective_id
    );
    assert_eq!(unknown_runtime.key.transition, unknown_trail.key.transition);
    assert_eq!(
        unknown_runtime.key.object_kind,
        unknown_trail.key.object_kind
    );
    assert_eq!(unknown_runtime.key.object_id, unknown_trail.key.object_id);
    assert_ne!(
        unknown_runtime.key.event_digest, unknown_trail.key.event_digest,
        "a receipt/event digest mismatch must prevent correlation"
    );

    let output = &fixture["expected_output"]["correlations"];
    let matched = array(&output["matched"], "matched links");
    let unknown = array(&output["unknown"], "unknown links");
    assert_eq!(matched.len(), 1);
    assert_eq!(unknown.len(), 2);
    assert_eq!(matched[0]["classification"], "matched");
    assert_eq!(matched[0]["runtime_record_id"], "runtime-07");
    assert_eq!(matched[0]["trail_record_id"], "trail-30");
    assert!(matched[0]["reason"].is_null());
    assert!(
        unknown
            .iter()
            .all(|link| link["classification"] == "unknown")
    );
    assert!(unknown.iter().any(|link| {
        link["runtime_record_id"] == "runtime-08"
            && link["trail_record_id"].is_null()
            && link["reason"] == "no_exact_trail_identity"
    }));
    assert!(unknown.iter().any(|link| {
        link["runtime_record_id"].is_null()
            && link["trail_record_id"] == "trail-32"
            && link["reason"] == "no_exact_runtime_identity"
    }));
}

#[test]
fn synthetic_d0_output_preserves_expected_counts_and_trail_authority() {
    let fixture = fixture();
    let output = &fixture["expected_output"];
    let counts = &output["mutation_attempts"]["counts"];
    assert_eq!(counts["stale_head_rejections"], 1);
    assert_eq!(
        counts["malformed_fault_classes"],
        json!([
            "sqlite_matcher",
            "decision_wrapper",
            "missing_structural_context"
        ])
    );
    assert_eq!(counts["duplicate_init_noops"], 1);

    assert_eq!(
        fixture["trail"]["prefix"],
        json!(["activate_objective", "install_map"])
    );
    let cycles = array(&fixture["trail"]["stage_cycles"], "Stage cycles");
    assert_eq!(cycles.len(), 5);
    let expected_cycle = json!([
        "add_route",
        "select_route",
        "start_attempt",
        "record_evidence",
        "seal_attempt",
        "decision"
    ]);
    for (index, cycle) in cycles.iter().enumerate() {
        assert_eq!(cycle["stage"], format!("S{}", index + 1));
        assert_eq!(cycle["transitions"], expected_cycle);
        assert_eq!(cycle["decision"], "accept");
        assert!(is_sha256(&cycle["evidence_baseline"]));
        assert_eq!(cycle["freshness"], "fresh");
    }
    assert_eq!(output["lifecycle_summary"]["accepted_stage_cycles"], 5);
    assert_eq!(
        output["subagent_summary"]["counts"]["verifiers_by_stage"],
        json!({"S1": 0, "S2": 0, "S3": 0, "S4": 0, "S5": 2})
    );
    assert_eq!(output["subagent_summary"]["counts"]["drivers"], 0);
    assert_eq!(output["subagent_summary"]["counts"]["stage_judges"], 0);
    assert_eq!(output["agent_path_conformance"]["status"], "nonconformant");
    assert_eq!(
        output["agent_path_conformance"]["rule"],
        "one_fresh_complete_judge_before_each_stage_review_decision"
    );
    assert_eq!(
        output["agent_path_conformance"]["required_stage_reviews"],
        5
    );
    assert_eq!(output["agent_path_conformance"]["observed_stage_judges"], 0);
    assert_eq!(
        output["agent_path_conformance"]["missing_judge_stages"],
        json!(["S1", "S2", "S3", "S4", "S5"])
    );
    assert_eq!(
        array(&output["deviations"], "deviations")
            .iter()
            .filter(|item| item["code"] == "missing_required_stage_judge")
            .count(),
        5
    );
    assert!(
        array(&output["residual_risks"], "residual risks")
            .iter()
            .any(|risk| {
                risk["code"] == "missing_required_stage_judge"
                    && risk["status"] == "open"
                    && risk["count"] == 5
            })
    );
    for task in array(&output["subagent_summary"]["tasks"], "Subagent tasks") {
        assert_eq!(task["spawn_status"], "completed");
        assert_eq!(task["final_status"], "completed");
        assert_eq!(task["close_status"], "completed");
        assert_eq!(task["consumed_before"], "record_evidence");
    }

    assert_eq!(fixture["authority"]["business_facts"], "trail");
    assert_eq!(output["trail_summary"]["authority"], "trail");
    assert_eq!(output["completion_summary"]["authority"], "trail");
    assert_eq!(
        output["completion_summary"]["objective_state"],
        fixture["trail"]["projected_final_state"]
    );
    assert_eq!(output["completion_summary"]["objective_state"], "achieved");
    assert_eq!(output["completion_summary"]["audit"], "healthy");
    assert_eq!(output["completion_summary"]["marker"], "present");

    assert_exact_keys(&fixture["judge"], &["reason", "status"], "source Judge");
    assert_eq!(fixture["judge"]["status"], "absent");
    assert_eq!(output["judge_summary"]["status"], "absent");
    for fabricated in ["freeze", "coverage", "disposition", "result"] {
        assert!(fixture["judge"].get(fabricated).is_none());
        assert!(output["judge_summary"].get(fabricated).is_none());
    }
}

#[test]
fn agent_facing_output_is_redacted_and_uses_local_digest_hints() {
    let fixture = fixture();
    let output = &fixture["expected_output"];
    let rendered = serde_json::to_string(output).unwrap();
    let locator = string(&fixture["input"]["session_locator"], "session locator");
    let native_session_identity = locator
        .rsplit('/')
        .next()
        .expect("synthetic locator has a label");

    assert!(
        !rendered.contains(locator),
        "output echoed the authorized session locator"
    );
    assert!(
        !rendered.contains("session_locator"),
        "output retained the locator field name"
    );
    assert!(
        !rendered.contains(&format!("\"{native_session_identity}\"")),
        "output retained the native session identity"
    );
    assert!(
        !contains_uuid_like(&rendered),
        "output contains a UUID-like session identity"
    );
    assert!(
        !contains_full_sha256(output),
        "Agent-facing output must not repeat full source digests"
    );
    assert_eq!(
        output["mutation_attempts"]["items"][0]["payload_digest"],
        output["mutation_attempts"]["items"][1]["payload_digest"],
        "a repeated digest must reuse one local id and hint"
    );

    for forbidden in [
        "/home/",
        "codex_internal_context",
        "raw_args",
        "raw_output",
        "full_prompt",
        "token_usage",
        "environment",
        "secret",
    ] {
        assert!(
            !rendered.contains(forbidden),
            "output leaked forbidden material: {forbidden}"
        );
    }

    let raw = read("plugins/mobius/runtime/tests/fixtures/session-audit-d0-v1.json");
    assert!(
        !contains_uuid_like(&raw),
        "synthetic fixture contains a UUID-like identity"
    );
    for forbidden in [
        "/home/",
        "codex_internal_context",
        "raw_args",
        "raw_output",
        "full_prompt",
    ] {
        assert!(
            !raw.contains(forbidden),
            "synthetic fixture leaked personal material: {forbidden}"
        );
    }
}

#[test]
fn colliding_short_digest_hints_fall_back_to_unambiguous_local_ids() {
    let mut references = DigestReferences::default();
    let first = references.render_text(
        "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa1234567",
        "digest-first".to_owned(),
    );
    let second = references.render_text(
        "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb1234567",
        "digest-second".to_owned(),
    );

    assert_eq!(first["display"], "sha256:…1234567");
    assert!(second["display"].is_null());
    assert_eq!(first["local_id"], "digest-first");
    assert_eq!(second["local_id"], "digest-second");
}
