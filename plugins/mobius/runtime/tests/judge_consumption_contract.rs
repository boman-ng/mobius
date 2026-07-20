use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Value, json};

#[derive(Debug, PartialEq, Eq)]
enum JudgeOutcome {
    Advice { disposition: String, external: bool },
    Inconclusive(&'static str),
    Unavailable(&'static str),
    Degraded(&'static str),
}

#[derive(Debug, PartialEq, Eq)]
enum StageAcceptGate {
    Allowed,
    Blocked(&'static str),
}

struct RequiredStageJudge<'a> {
    created_after_review_freeze: bool,
    outcome: &'a JudgeOutcome,
    findings_resolved_by_main: bool,
}

fn gate_stage_accept(
    required_judge: Option<RequiredStageJudge<'_>>,
    main_review_closed: bool,
) -> StageAcceptGate {
    let Some(required_judge) = required_judge else {
        return StageAcceptGate::Blocked("required_stage_judge_absent");
    };
    if !required_judge.created_after_review_freeze {
        return StageAcceptGate::Blocked("required_stage_judge_stale");
    }
    if !main_review_closed {
        return StageAcceptGate::Blocked("main_review_incomplete");
    }

    match required_judge.outcome {
        JudgeOutcome::Advice { disposition, .. }
            if disposition == "proceed" || required_judge.findings_resolved_by_main =>
        {
            StageAcceptGate::Allowed
        }
        JudgeOutcome::Advice { .. } => {
            StageAcceptGate::Blocked("required_stage_judge_findings_unresolved")
        }
        JudgeOutcome::Inconclusive(_) => {
            StageAcceptGate::Blocked("required_stage_judge_inconclusive")
        }
        JudgeOutcome::Unavailable(_) => {
            StageAcceptGate::Blocked("required_stage_judge_unavailable")
        }
        JudgeOutcome::Degraded(_) => StageAcceptGate::Blocked("required_stage_judge_degraded"),
    }
}

const FREEZE_A: &str = "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const FREEZE_B: &str = "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

#[derive(Clone, Debug, Eq, PartialEq)]
struct FreezeIdentity {
    method: String,
    value: String,
}

fn judge_task() -> Value {
    json!({
        "role": "judge",
        "background": {
            "why_now": "one frozen semantic question remains after deterministic Review closure",
            "current_state": ["the main Agent has closed the complete Packet identity graph"],
            "confirmed_facts": [],
            "materials": [
                {"id": "BM1", "locator": "frozen packet", "purpose": "review target"},
                {"id": "BM2", "locator": "frozen evidence bundle", "purpose": "supporting material"}
            ],
            "assumptions_to_check": [{"id": "BA1", "assumption": "the frozen material satisfies the criterion"}]
        },
        "objectives": [{"id": "O1", "objective": "challenge the frozen material", "priority": "must"}],
        "boundaries": {"forbidden": [], "focus": []},
        "role_input": {
            "materials": [
                {
                    "id": "JM1",
                    "locator": "frozen packet",
                    "purpose": "answer the bounded review question",
                    "freeze": {"method": "content_digest", "value": FREEZE_A}
                },
                {
                    "id": "JM2",
                    "locator": "frozen evidence bundle",
                    "purpose": "verify the supporting evidence",
                    "freeze": {"method": "content_digest", "value": FREEZE_B}
                }
            ],
            "questions": [
                {"id": "JQ1", "question": "does the material support the conclusion?", "material_ids": ["JM1"], "required_coverage": "complete material"},
                {"id": "JQ2", "question": "does the evidence bundle support the material?", "material_ids": ["JM2"], "required_coverage": "complete material"}
            ],
            "criteria": [{"id": "JC1", "criterion": "all counterevidence is addressed", "material_ids": ["JM1", "JM2"], "required_coverage": "complete material"}],
            "known_risks": [{"id": "JR1", "risk": "a semantic contradiction remains", "material_ids": ["JM1", "JM2"], "required_coverage": "complete material"}],
            "disposition_options": ["proceed", "revise"]
        },
        "output_format": {
            "representation": "json",
            "template": "complete common result plus one Judge role_output",
            "constraints": [],
            "result_budget": {"max_public_result_bytes": 8192}
        },
        "done_when": [{"id": "D1", "condition": "all questions, criteria, and risks are assessed", "evidence_required": ["material coverage"]}]
    })
}

fn judge_result() -> Value {
    json!({
        "status": "completed",
        "summary": "the frozen material was reviewed",
        "objective_results": [{"objective_id": "O1", "status": "achieved", "result": "bounded advisory review completed", "evidence": ["JM1", "JM2"]}],
        "assumption_results": [{"assumption_id": "BA1", "assessment": "confirmed", "impact": "advisory only", "evidence": ["JM1", "JM2"]}],
        "done_when_results": [{"done_when_id": "D1", "status": "satisfied", "evidence": ["JM1", "JM2"], "reason": "all required coverage was inspected"}],
        "boundary_compliance": {"status": "compliant", "violations": []},
        "effects": [],
        "artifacts": [],
        "uncertainties": [],
        "blockers": [],
        "overflow": {"omitted_items": 0, "artifact_ids": [], "reason": "none"},
        "role_output": {
            "material_results": [
                {
                    "material_id": "JM1",
                    "status": "reviewed",
                    "freeze_check": {"status": "matched", "observed": FREEZE_A},
                    "coverage": "complete material",
                    "evidence": ["JM1"]
                },
                {
                    "material_id": "JM2",
                    "status": "reviewed",
                    "freeze_check": {"status": "matched", "observed": FREEZE_B},
                    "coverage": "complete material",
                    "evidence": ["JM2"]
                }
            ],
            "answers": [
                {"question_id": "JQ1", "assessment": "answered", "answer": "the bounded conclusion is supported", "material_ids": ["JM1"], "coverage_status": "complete", "evidence": ["JM1"]},
                {"question_id": "JQ2", "assessment": "answered", "answer": "the supporting evidence is complete", "material_ids": ["JM2"], "coverage_status": "complete", "evidence": ["JM2"]}
            ],
            "criterion_assessments": [{"criterion_id": "JC1", "assessment": "satisfied", "material_ids": ["JM1", "JM2"], "coverage_status": "complete", "evidence": ["JM1", "JM2"], "reason": "counterevidence is addressed"}],
            "risk_assessments": [{"risk_id": "JR1", "assessment": "mitigated", "material_ids": ["JM1", "JM2"], "coverage_status": "complete", "evidence": ["JM1", "JM2"]}],
            "findings": [],
            "recommended_disposition": "proceed",
            "recommendations": [{"recommendation": "main Agent may proceed after its own Review", "reason": "bounded advice", "evidence": ["JM1", "JM2"]}]
        }
    })
}

fn ids(value: &Value, field: &str) -> Option<BTreeSet<String>> {
    let mut found = BTreeSet::new();
    for item in value.as_array()? {
        let id = item[field].as_str().filter(|id| !id.is_empty())?;
        if !found.insert(id.to_owned()) {
            return None;
        }
    }
    Some(found)
}

fn closed(task: &Value, result: &Value, task_field: &str, result_field: &str, id: &str) -> bool {
    ids(&task[task_field], "id") == ids(&result[result_field], id)
}

fn string_set(value: &Value) -> Option<BTreeSet<String>> {
    let mut found = BTreeSet::new();
    for item in value.as_array()? {
        let item = item.as_str().filter(|item| !item.is_empty())?;
        if !found.insert(item.to_owned()) {
            return None;
        }
    }
    Some(found)
}

fn exact_fields(value: &Value, expected: &[&str]) -> bool {
    value.as_object().is_some_and(|object| {
        object.keys().map(String::as_str).collect::<BTreeSet<_>>()
            == expected.iter().copied().collect::<BTreeSet<_>>()
    })
}

fn nonempty_array(value: &Value) -> bool {
    value.as_array().is_some_and(|items| !items.is_empty())
}

fn every_exact(value: &Value, expected: &[&str]) -> bool {
    value
        .as_array()
        .is_some_and(|items| items.iter().all(|item| exact_fields(item, expected)))
}

fn all_nonempty_text(value: &Value, field: &str) -> bool {
    value.as_array().is_some_and(|items| {
        items.iter().all(|item| {
            item[field]
                .as_str()
                .is_some_and(|value| !value.trim().is_empty())
        })
    })
}

fn allowed(value: &Value, options: &[&str]) -> bool {
    value.as_str().is_some_and(|value| options.contains(&value))
}

fn is_sha256(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(|hex| {
        hex.len() == 64
            && hex
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    })
}

fn valid_freeze(method: &str, value: &str) -> bool {
    match method {
        "content_digest" => is_sha256(value),
        "inline" | "immutable_version" | "immutable_object_id" => !value.trim().is_empty(),
        _ => false,
    }
}

fn current_freezes() -> BTreeMap<String, FreezeIdentity> {
    BTreeMap::from([
        (
            "JM1".to_owned(),
            FreezeIdentity {
                method: "content_digest".to_owned(),
                value: FREEZE_A.to_owned(),
            },
        ),
        (
            "JM2".to_owned(),
            FreezeIdentity {
                method: "content_digest".to_owned(),
                value: FREEZE_B.to_owned(),
            },
        ),
    ])
}

fn coverage_group_complete(
    task_items: &Value,
    result_items: &Value,
    task_id_field: &str,
    result_id_field: &str,
    material_coverage: &BTreeMap<String, String>,
    complete_materials: &BTreeSet<String>,
    allowed_assessments: &[&str],
) -> Option<bool> {
    let task_items = task_items.as_array()?;
    let result_items = result_items.as_array()?;
    if task_items.is_empty() || result_items.is_empty() {
        return None;
    }
    let valid_materials = material_coverage.keys().cloned().collect::<BTreeSet<_>>();
    let mut complete = true;

    for task_item in task_items {
        let task_id = task_item[task_id_field].as_str()?;
        let required_coverage = task_item["required_coverage"]
            .as_str()
            .filter(|coverage| !coverage.is_empty())?;
        let referenced = string_set(&task_item["material_ids"])
            .filter(|ids| !ids.is_empty() && ids.is_subset(&valid_materials))?;
        let result_item = result_items
            .iter()
            .find(|item| item[result_id_field].as_str() == Some(task_id))?;
        if string_set(&result_item["material_ids"]).as_ref() != Some(&referenced)
            || string_set(&result_item["evidence"]).as_ref() != Some(&referenced)
        {
            return None;
        }
        let coverage_status = result_item["coverage_status"].as_str()?;
        let assessment = result_item["assessment"].as_str()?;
        if !["complete", "partial", "unverifiable"].contains(&coverage_status)
            || !allowed_assessments.contains(&assessment)
        {
            return None;
        }
        let required_material_coverage = referenced.iter().all(|material_id| {
            material_coverage.get(material_id).map(String::as_str) == Some(required_coverage)
        });
        let necessary_materials_complete = referenced.is_subset(complete_materials);
        if (coverage_status != "complete"
            || !required_material_coverage
            || !necessary_materials_complete)
            && assessment != "inconclusive"
        {
            return None;
        }
        complete &= coverage_status == "complete"
            && assessment != "inconclusive"
            && required_material_coverage
            && necessary_materials_complete;
    }

    Some(complete)
}

fn advisory_references_valid(
    role_output: &Value,
    material_ids: &BTreeSet<String>,
    criterion_ids: &BTreeSet<String>,
    evidence_ids: &BTreeSet<String>,
) -> bool {
    let Some(findings) = role_output["findings"].as_array() else {
        return false;
    };
    for finding in findings {
        if !allowed(&finding["severity"], &["minor", "major", "blocking"])
            || finding["finding"]
                .as_str()
                .is_none_or(|value| value.trim().is_empty())
        {
            return false;
        }
        let Some(referenced_criteria) = string_set(&finding["criterion_ids"])
            .filter(|ids| !ids.is_empty() && ids.is_subset(criterion_ids))
        else {
            return false;
        };
        let Some(_referenced_materials) = string_set(&finding["material_ids"])
            .filter(|ids| !ids.is_empty() && ids.is_subset(material_ids))
        else {
            return false;
        };
        if referenced_criteria.is_empty()
            || string_set(&finding["evidence"])
                .is_none_or(|ids| ids.is_empty() || !ids.is_subset(evidence_ids))
        {
            return false;
        }
    }

    let Some(recommendations) = role_output["recommendations"].as_array() else {
        return false;
    };
    recommendations.iter().all(|recommendation| {
        recommendation["recommendation"]
            .as_str()
            .is_some_and(|value| !value.trim().is_empty())
            && recommendation["reason"]
                .as_str()
                .is_some_and(|value| !value.trim().is_empty())
            && string_set(&recommendation["evidence"])
                .is_some_and(|ids| ids.is_subset(evidence_ids))
    })
}

fn common_evidence_references_valid(result: &Value, evidence_ids: &BTreeSet<String>) -> bool {
    [
        &result["objective_results"],
        &result["assumption_results"],
        &result["done_when_results"],
    ]
    .into_iter()
    .all(|items| {
        items.as_array().is_some_and(|items| {
            items.iter().all(|item| {
                string_set(&item["evidence"]).is_some_and(|ids| ids.is_subset(evidence_ids))
            })
        })
    })
}

fn common_result_valid(task: &Value, result: &Value) -> bool {
    let expected_fields = [
        "status",
        "summary",
        "objective_results",
        "assumption_results",
        "done_when_results",
        "boundary_compliance",
        "effects",
        "artifacts",
        "uncertainties",
        "blockers",
        "overflow",
        "role_output",
    ];
    if !exact_fields(result, &expected_fields)
        || result["status"] != "completed"
        || result["summary"]
            .as_str()
            .is_none_or(|value| value.trim().is_empty())
        || !nonempty_array(&task["objectives"])
        || !nonempty_array(&task["done_when"])
        || !closed(
            task,
            result,
            "objectives",
            "objective_results",
            "objective_id",
        )
        || !closed(
            &task["background"],
            result,
            "assumptions_to_check",
            "assumption_results",
            "assumption_id",
        )
        || !closed(
            task,
            result,
            "done_when",
            "done_when_results",
            "done_when_id",
        )
        || !every_exact(
            &result["objective_results"],
            &["objective_id", "status", "result", "evidence"],
        )
        || !result["objective_results"].as_array().is_some_and(|items| {
            items.iter().all(|item| {
                allowed(
                    &item["status"],
                    &["achieved", "partial", "blocked", "failed"],
                ) && item["result"]
                    .as_str()
                    .is_some_and(|value| !value.trim().is_empty())
                    && string_set(&item["evidence"]).is_some()
            })
        })
        || !every_exact(
            &result["assumption_results"],
            &["assumption_id", "assessment", "impact", "evidence"],
        )
        || !result["assumption_results"]
            .as_array()
            .is_some_and(|items| {
                items.iter().all(|item| {
                    allowed(
                        &item["assessment"],
                        &["confirmed", "contradicted", "inconclusive", "not_evaluated"],
                    ) && item["impact"]
                        .as_str()
                        .is_some_and(|value| !value.trim().is_empty())
                        && string_set(&item["evidence"]).is_some()
                })
            })
        || !every_exact(
            &result["done_when_results"],
            &["done_when_id", "status", "evidence", "reason"],
        )
        || !result["done_when_results"].as_array().is_some_and(|items| {
            items.iter().all(|item| {
                allowed(
                    &item["status"],
                    &["satisfied", "unsatisfied", "unknown", "not_evaluated"],
                ) && item["reason"]
                    .as_str()
                    .is_some_and(|value| !value.trim().is_empty())
                    && string_set(&item["evidence"]).is_some()
            })
        })
        || !exact_fields(&result["boundary_compliance"], &["status", "violations"])
        || result["boundary_compliance"]["status"] != "compliant"
        || !result["boundary_compliance"]["violations"]
            .as_array()
            .is_some_and(Vec::is_empty)
        || !result["effects"].as_array().is_some_and(Vec::is_empty)
        || !result["artifacts"].as_array().is_some_and(|items| {
            every_exact(&result["artifacts"], &["id", "locator", "description"])
                && ids(&result["artifacts"], "id").is_some()
                && items.iter().all(|item| {
                    item["locator"]
                        .as_str()
                        .is_some_and(|value| !value.trim().is_empty())
                        && item["description"]
                            .as_str()
                            .is_some_and(|value| !value.trim().is_empty())
                })
        })
        || !result["uncertainties"]
            .as_array()
            .is_some_and(Vec::is_empty)
        || !result["blockers"].as_array().is_some_and(Vec::is_empty)
        || !exact_fields(
            &result["overflow"],
            &["omitted_items", "artifact_ids", "reason"],
        )
        || result["overflow"]["omitted_items"] != 0
        || !result["overflow"]["artifact_ids"]
            .as_array()
            .is_some_and(Vec::is_empty)
        || result["overflow"]["reason"] != "none"
    {
        return false;
    }

    result["objective_results"]
        .as_array()
        .is_some_and(|items| items.iter().all(|item| item["status"] == "achieved"))
        && result["done_when_results"]
            .as_array()
            .is_some_and(|items| items.iter().all(|item| item["status"] == "satisfied"))
}

fn consume_judge(
    task: &Value,
    native_status: &str,
    final_output: Option<&Value>,
    current_freezes: &BTreeMap<String, FreezeIdentity>,
    external_requested: bool,
    runtime_advertised_external: bool,
) -> JudgeOutcome {
    if external_requested && !runtime_advertised_external {
        return JudgeOutcome::Unavailable("external_profile_not_advertised");
    }
    if native_status != "completed" {
        return JudgeOutcome::Degraded("native_execution_unavailable");
    }
    let Some(result) = final_output else {
        return JudgeOutcome::Degraded("missing_final_output");
    };
    let Some(budget) = task["output_format"]["result_budget"]["max_public_result_bytes"]
        .as_u64()
        .filter(|value| *value > 0)
    else {
        return JudgeOutcome::Degraded("invalid_result_budget");
    };
    if serde_json::to_vec(result).map_or(true, |bytes| bytes.len() as u64 > budget) {
        return JudgeOutcome::Degraded("result_budget_exceeded");
    }
    if !common_result_valid(task, result) {
        return JudgeOutcome::Degraded("invalid_result_envelope");
    }

    let role_input = &task["role_input"];
    let role_output = &result["role_output"];
    if !exact_fields(
        role_input,
        &[
            "materials",
            "questions",
            "criteria",
            "known_risks",
            "disposition_options",
        ],
    ) || !exact_fields(
        role_output,
        &[
            "material_results",
            "answers",
            "criterion_assessments",
            "risk_assessments",
            "findings",
            "recommended_disposition",
            "recommendations",
        ],
    ) || !nonempty_array(&role_input["materials"])
        || !nonempty_array(&role_input["questions"])
        || !nonempty_array(&role_input["criteria"])
        || !nonempty_array(&role_input["disposition_options"])
        || !every_exact(
            &role_input["materials"],
            &["id", "locator", "purpose", "freeze"],
        )
        || !role_input["materials"].as_array().is_some_and(|items| {
            items.iter().all(|item| {
                exact_fields(&item["freeze"], &["method", "value"])
                    && item["locator"]
                        .as_str()
                        .is_some_and(|value| !value.trim().is_empty())
                    && item["purpose"]
                        .as_str()
                        .is_some_and(|value| !value.trim().is_empty())
            })
        })
        || !every_exact(
            &role_input["questions"],
            &["id", "question", "material_ids", "required_coverage"],
        )
        || !every_exact(
            &role_input["criteria"],
            &["id", "criterion", "material_ids", "required_coverage"],
        )
        || !every_exact(
            &role_input["known_risks"],
            &["id", "risk", "material_ids", "required_coverage"],
        )
        || !all_nonempty_text(&role_input["questions"], "question")
        || !all_nonempty_text(&role_input["criteria"], "criterion")
        || !all_nonempty_text(&role_input["known_risks"], "risk")
        || string_set(&role_input["disposition_options"]).is_none_or(|options| options.is_empty())
        || !every_exact(
            &role_output["material_results"],
            &[
                "material_id",
                "status",
                "freeze_check",
                "coverage",
                "evidence",
            ],
        )
        || !role_output["material_results"]
            .as_array()
            .is_some_and(|items| {
                items
                    .iter()
                    .all(|item| exact_fields(&item["freeze_check"], &["status", "observed"]))
            })
        || !every_exact(
            &role_output["answers"],
            &[
                "question_id",
                "assessment",
                "answer",
                "material_ids",
                "coverage_status",
                "evidence",
            ],
        )
        || !every_exact(
            &role_output["criterion_assessments"],
            &[
                "criterion_id",
                "assessment",
                "material_ids",
                "coverage_status",
                "evidence",
                "reason",
            ],
        )
        || !every_exact(
            &role_output["risk_assessments"],
            &[
                "risk_id",
                "assessment",
                "material_ids",
                "coverage_status",
                "evidence",
            ],
        )
        || !every_exact(
            &role_output["findings"],
            &[
                "severity",
                "finding",
                "criterion_ids",
                "material_ids",
                "evidence",
            ],
        )
        || !every_exact(
            &role_output["recommendations"],
            &["recommendation", "reason", "evidence"],
        )
        || !all_nonempty_text(&role_output["answers"], "answer")
        || !all_nonempty_text(&role_output["criterion_assessments"], "reason")
    {
        return JudgeOutcome::Degraded("invalid_judge_role_envelope");
    }
    if !closed(
        role_input,
        role_output,
        "materials",
        "material_results",
        "material_id",
    ) || !closed(
        role_input,
        role_output,
        "questions",
        "answers",
        "question_id",
    ) || !closed(
        role_input,
        role_output,
        "criteria",
        "criterion_assessments",
        "criterion_id",
    ) || !closed(
        role_input,
        role_output,
        "known_risks",
        "risk_assessments",
        "risk_id",
    ) {
        return JudgeOutcome::Degraded("judge_role_output_incomplete");
    }

    let Some(material_ids) = ids(&role_input["materials"], "id") else {
        return JudgeOutcome::Degraded("invalid_judge_materials");
    };
    let mut evidence_ids = material_ids.clone();
    let Some(artifact_ids) = ids(&result["artifacts"], "id") else {
        return JudgeOutcome::Degraded("invalid_result_envelope");
    };
    evidence_ids.extend(artifact_ids);
    if !common_evidence_references_valid(result, &evidence_ids) {
        return JudgeOutcome::Degraded("invalid_result_evidence_references");
    }
    if current_freezes.keys().cloned().collect::<BTreeSet<_>>() != material_ids {
        return JudgeOutcome::Inconclusive("stale_task_material");
    }

    let Some(task_materials) = role_input["materials"].as_array() else {
        return JudgeOutcome::Degraded("invalid_judge_materials");
    };
    let Some(material_results) = role_output["material_results"].as_array() else {
        return JudgeOutcome::Degraded("judge_role_output_incomplete");
    };
    let mut complete = true;
    let mut material_coverage = BTreeMap::new();
    let mut complete_materials = BTreeSet::new();
    for task_material in task_materials {
        let Some(material_id) = task_material["id"].as_str() else {
            return JudgeOutcome::Degraded("invalid_judge_materials");
        };
        let Some(freeze_method) = task_material["freeze"]["method"].as_str() else {
            return JudgeOutcome::Degraded("invalid_judge_materials");
        };
        let Some(declared_freeze) = task_material["freeze"]["value"].as_str() else {
            return JudgeOutcome::Degraded("invalid_judge_materials");
        };
        if !valid_freeze(freeze_method, declared_freeze) {
            return JudgeOutcome::Degraded("invalid_judge_materials");
        }
        if current_freezes.get(material_id)
            != Some(&FreezeIdentity {
                method: freeze_method.to_owned(),
                value: declared_freeze.to_owned(),
            })
        {
            return JudgeOutcome::Inconclusive("stale_task_material");
        }

        let Some(material_result) = material_results
            .iter()
            .find(|item| item["material_id"].as_str() == Some(material_id))
        else {
            return JudgeOutcome::Degraded("judge_role_output_incomplete");
        };
        let expected_evidence = BTreeSet::from([material_id.to_owned()]);
        if string_set(&material_result["evidence"]).as_ref() != Some(&expected_evidence) {
            return JudgeOutcome::Degraded("invalid_judge_material_evidence");
        }
        let Some(status) = material_result["status"].as_str() else {
            return JudgeOutcome::Degraded("invalid_judge_material_status");
        };
        let Some(freeze_status) = material_result["freeze_check"]["status"].as_str() else {
            return JudgeOutcome::Degraded("invalid_judge_material_status");
        };
        let Some(observed) = material_result["freeze_check"]["observed"]
            .as_str()
            .filter(|value| !value.trim().is_empty())
        else {
            return JudgeOutcome::Degraded("invalid_judge_material_status");
        };
        let Some(coverage) = material_result["coverage"]
            .as_str()
            .filter(|value| !value.trim().is_empty())
        else {
            return JudgeOutcome::Degraded("invalid_judge_material_status");
        };
        let status_is_coherent = match status {
            "reviewed" | "partial" => freeze_status == "matched" && observed == declared_freeze,
            "stale" => freeze_status == "mismatched" && observed != declared_freeze,
            "unverifiable" | "inaccessible" => freeze_status == "unverifiable",
            _ => false,
        };
        if !status_is_coherent {
            return JudgeOutcome::Degraded("invalid_judge_material_status");
        }
        material_coverage.insert(material_id.to_owned(), coverage.to_owned());
        if status == "reviewed" {
            complete_materials.insert(material_id.to_owned());
        }
        complete &= status == "reviewed";
    }

    for (task_field, result_field, task_id, result_id) in [
        ("questions", "answers", "id", "question_id"),
        ("criteria", "criterion_assessments", "id", "criterion_id"),
        ("known_risks", "risk_assessments", "id", "risk_id"),
    ] {
        if task_field == "known_risks"
            && role_input[task_field].as_array().is_some_and(Vec::is_empty)
        {
            continue;
        }
        let Some(group_complete) = coverage_group_complete(
            &role_input[task_field],
            &role_output[result_field],
            task_id,
            result_id,
            &material_coverage,
            &complete_materials,
            match result_field {
                "answers" => &["answered", "inconclusive"],
                "criterion_assessments" => &["satisfied", "unsatisfied", "inconclusive"],
                "risk_assessments" => &["observed", "mitigated", "unsupported", "inconclusive"],
                _ => unreachable!(),
            },
        ) else {
            return JudgeOutcome::Degraded("invalid_judge_material_references");
        };
        complete &= group_complete;
    }

    let Some(criterion_ids) = ids(&role_input["criteria"], "id") else {
        return JudgeOutcome::Degraded("invalid_judge_material_references");
    };
    if !advisory_references_valid(role_output, &material_ids, &criterion_ids, &evidence_ids) {
        return JudgeOutcome::Degraded("invalid_judge_advisory_references");
    }

    if !complete {
        if role_output["recommended_disposition"] == "inconclusive" {
            return JudgeOutcome::Inconclusive("material_or_coverage_incomplete");
        }
        return JudgeOutcome::Degraded("determinate_advice_bypassed_freeze_gate");
    }

    if role_output["recommended_disposition"] == "inconclusive" {
        return JudgeOutcome::Inconclusive("judge_declined_determinate_disposition");
    }

    let Some(disposition) = role_output["recommended_disposition"]
        .as_str()
        .filter(|value| {
            role_input["disposition_options"]
                .as_array()
                .is_some_and(|options| options.iter().any(|option| option == *value))
        })
    else {
        return JudgeOutcome::Degraded("invalid_judge_disposition");
    };
    JudgeOutcome::Advice {
        disposition: disposition.to_owned(),
        external: external_requested && runtime_advertised_external,
    }
}

fn make_inconclusive(result: &mut Value, material_status: &str, freeze_status: &str) {
    for material in result["role_output"]["material_results"]
        .as_array_mut()
        .unwrap()
    {
        material["status"] = json!(material_status);
        material["freeze_check"]["status"] = json!(freeze_status);
        if freeze_status == "mismatched" {
            material["freeze_check"]["observed"] =
                json!("sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc");
        } else if freeze_status == "unverifiable" {
            material["freeze_check"]["observed"] = json!("freeze could not be checked");
        }
        material["coverage"] = json!("unverifiable");
    }
    for field in ["answers", "criterion_assessments", "risk_assessments"] {
        for assessment in result["role_output"][field].as_array_mut().unwrap() {
            assessment["assessment"] = json!("inconclusive");
            assessment["coverage_status"] = json!("unverifiable");
        }
    }
    result["role_output"]["recommended_disposition"] = json!("inconclusive");
}

#[test]
fn every_matched_complete_judge_material_returns_advice_only() {
    let task = judge_task();
    let result = judge_result();
    assert_eq!(
        consume_judge(
            &task,
            "completed",
            Some(&result),
            &current_freezes(),
            false,
            false,
        ),
        JudgeOutcome::Advice {
            disposition: "proceed".to_owned(),
            external: false
        }
    );
    for forbidden in ["evidence", "decision", "command", "transition", "proof"] {
        assert!(
            result["role_output"].get(forbidden).is_none(),
            "Judge advice must not contain downstream {forbidden}"
        );
    }
}

#[test]
fn every_stage_accept_requires_one_fresh_complete_judge_and_main_review() {
    let proceed = JudgeOutcome::Advice {
        disposition: "proceed".to_owned(),
        external: false,
    };
    assert_eq!(
        gate_stage_accept(
            Some(RequiredStageJudge {
                created_after_review_freeze: true,
                outcome: &proceed,
                findings_resolved_by_main: false,
            }),
            true,
        ),
        StageAcceptGate::Allowed
    );

    assert_eq!(
        gate_stage_accept(None, true),
        StageAcceptGate::Blocked("required_stage_judge_absent")
    );
    assert_eq!(
        gate_stage_accept(
            Some(RequiredStageJudge {
                created_after_review_freeze: false,
                outcome: &proceed,
                findings_resolved_by_main: false,
            }),
            true,
        ),
        StageAcceptGate::Blocked("required_stage_judge_stale")
    );
    assert_eq!(
        gate_stage_accept(
            Some(RequiredStageJudge {
                created_after_review_freeze: true,
                outcome: &proceed,
                findings_resolved_by_main: false,
            }),
            false,
        ),
        StageAcceptGate::Blocked("main_review_incomplete")
    );
}

#[test]
fn unavailable_inconclusive_degraded_or_unresolved_judge_blocks_stage_accept() {
    let cases = [
        (
            JudgeOutcome::Unavailable("native_execution_unavailable"),
            "required_stage_judge_unavailable",
        ),
        (
            JudgeOutcome::Inconclusive("material_or_coverage_incomplete"),
            "required_stage_judge_inconclusive",
        ),
        (
            JudgeOutcome::Degraded("invalid_result_envelope"),
            "required_stage_judge_degraded",
        ),
    ];
    for (outcome, reason) in &cases {
        assert_eq!(
            gate_stage_accept(
                Some(RequiredStageJudge {
                    created_after_review_freeze: true,
                    outcome,
                    findings_resolved_by_main: false,
                }),
                true,
            ),
            StageAcceptGate::Blocked(reason)
        );
    }

    let revise = JudgeOutcome::Advice {
        disposition: "revise".to_owned(),
        external: false,
    };
    assert_eq!(
        gate_stage_accept(
            Some(RequiredStageJudge {
                created_after_review_freeze: true,
                outcome: &revise,
                findings_resolved_by_main: false,
            }),
            true,
        ),
        StageAcceptGate::Blocked("required_stage_judge_findings_unresolved")
    );
    assert_eq!(
        gate_stage_accept(
            Some(RequiredStageJudge {
                created_after_review_freeze: true,
                outcome: &revise,
                findings_resolved_by_main: true,
            }),
            true,
        ),
        StageAcceptGate::Allowed,
        "Judge advice remains advisory after the main Agent resolves every finding"
    );
}

#[test]
fn partial_stale_and_unverifiable_material_are_inconclusive() {
    let task = judge_task();
    for (material, freeze) in [
        ("partial", "matched"),
        ("stale", "mismatched"),
        ("unverifiable", "unverifiable"),
        ("inaccessible", "unverifiable"),
    ] {
        let mut result = judge_result();
        make_inconclusive(&mut result, material, freeze);
        assert_eq!(
            consume_judge(
                &task,
                "completed",
                Some(&result),
                &current_freezes(),
                false,
                false,
            ),
            JudgeOutcome::Inconclusive("material_or_coverage_incomplete")
        );
    }

    let mut invalid = judge_result();
    make_inconclusive(&mut invalid, "partial", "matched");
    invalid["role_output"]["recommended_disposition"] = json!("proceed");
    assert_eq!(
        consume_judge(
            &task,
            "completed",
            Some(&invalid),
            &current_freezes(),
            false,
            false,
        ),
        JudgeOutcome::Degraded("determinate_advice_bypassed_freeze_gate")
    );
}

#[test]
fn runtime_failures_and_external_profile_absence_never_synthesize_success() {
    let task = judge_task();
    let result = judge_result();
    for status in [
        "timeout",
        "interrupted",
        "spawn_failure",
        "configuration_failure",
        "permission_failure",
    ] {
        assert_eq!(
            consume_judge(
                &task,
                status,
                Some(&result),
                &current_freezes(),
                false,
                false,
            ),
            JudgeOutcome::Degraded("native_execution_unavailable")
        );
    }
    assert_eq!(
        consume_judge(&task, "completed", None, &current_freezes(), false, false,),
        JudgeOutcome::Degraded("missing_final_output")
    );
    assert_eq!(
        consume_judge(
            &task,
            "completed",
            Some(&result),
            &current_freezes(),
            true,
            false,
        ),
        JudgeOutcome::Unavailable("external_profile_not_advertised")
    );
    assert_eq!(
        consume_judge(
            &task,
            "completed",
            Some(&result),
            &current_freezes(),
            true,
            true,
        ),
        JudgeOutcome::Advice {
            disposition: "proceed".to_owned(),
            external: true
        }
    );
}

#[test]
fn every_material_participates_in_freeze_closure_and_coverage() {
    let task = judge_task();
    let result = judge_result();

    let mut stale_second = current_freezes();
    stale_second.insert(
        "JM2".to_owned(),
        FreezeIdentity {
            method: "content_digest".to_owned(),
            value: "sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"
                .to_owned(),
        },
    );
    assert_eq!(
        consume_judge(
            &task,
            "completed",
            Some(&result),
            &stale_second,
            false,
            false,
        ),
        JudgeOutcome::Inconclusive("stale_task_material")
    );

    let mut missing_second = result.clone();
    missing_second["role_output"]["material_results"]
        .as_array_mut()
        .unwrap()
        .pop();
    assert_eq!(
        consume_judge(
            &task,
            "completed",
            Some(&missing_second),
            &current_freezes(),
            false,
            false,
        ),
        JudgeOutcome::Degraded("judge_role_output_incomplete")
    );

    let mut extra = result.clone();
    let mut extra_material = extra["role_output"]["material_results"][1].clone();
    extra_material["material_id"] = json!("JM3");
    extra["role_output"]["material_results"]
        .as_array_mut()
        .unwrap()
        .push(extra_material);
    assert_eq!(
        consume_judge(
            &task,
            "completed",
            Some(&extra),
            &current_freezes(),
            false,
            false,
        ),
        JudgeOutcome::Degraded("judge_role_output_incomplete")
    );

    let mut duplicate = result.clone();
    let duplicate_material = duplicate["role_output"]["material_results"][1].clone();
    duplicate["role_output"]["material_results"]
        .as_array_mut()
        .unwrap()
        .push(duplicate_material);
    assert_eq!(
        consume_judge(
            &task,
            "completed",
            Some(&duplicate),
            &current_freezes(),
            false,
            false,
        ),
        JudgeOutcome::Degraded("judge_role_output_incomplete")
    );

    let mut partial_second = result.clone();
    partial_second["role_output"]["material_results"][1]["status"] = json!("partial");
    partial_second["role_output"]["material_results"][1]["coverage"] = json!("partial");
    assert_eq!(
        consume_judge(
            &task,
            "completed",
            Some(&partial_second),
            &current_freezes(),
            false,
            false,
        ),
        JudgeOutcome::Degraded("invalid_judge_material_references")
    );

    let mut partial_second_question = result.clone();
    partial_second_question["role_output"]["answers"][1]["coverage_status"] = json!("partial");
    assert_eq!(
        consume_judge(
            &task,
            "completed",
            Some(&partial_second_question),
            &current_freezes(),
            false,
            false,
        ),
        JudgeOutcome::Degraded("invalid_judge_material_references")
    );
}

#[test]
fn every_generic_freeze_method_is_checked_exactly() {
    for (method, value) in [
        ("inline", "fixed inline material"),
        ("content_digest", FREEZE_A),
        ("immutable_version", "release-v1"),
        ("immutable_object_id", "object-42"),
    ] {
        let mut task = judge_task();
        let mut result = judge_result();
        let mut freezes = current_freezes();
        task["role_input"]["materials"][0]["freeze"]["method"] = json!(method);
        task["role_input"]["materials"][0]["freeze"]["value"] = json!(value);
        result["role_output"]["material_results"][0]["freeze_check"]["observed"] = json!(value);
        freezes.insert(
            "JM1".to_owned(),
            FreezeIdentity {
                method: method.to_owned(),
                value: value.to_owned(),
            },
        );
        assert_eq!(
            consume_judge(&task, "completed", Some(&result), &freezes, false, false,),
            JudgeOutcome::Advice {
                disposition: "proceed".to_owned(),
                external: false,
            },
            "freeze method {method} was not consumed through the generic gate"
        );
    }

    let mut invalid_task = judge_task();
    invalid_task["role_input"]["materials"][0]["freeze"]["method"] = json!("mutable_url");
    assert_eq!(
        consume_judge(
            &invalid_task,
            "completed",
            Some(&judge_result()),
            &current_freezes(),
            false,
            false,
        ),
        JudgeOutcome::Degraded("invalid_judge_materials")
    );
}

#[test]
fn status_coverage_and_advisory_references_are_closed() {
    let task = judge_task();

    let mut no_risk_task = task.clone();
    let mut no_risk_result = judge_result();
    no_risk_task["role_input"]["known_risks"] = json!([]);
    no_risk_result["role_output"]["risk_assessments"] = json!([]);
    assert!(matches!(
        consume_judge(
            &no_risk_task,
            "completed",
            Some(&no_risk_result),
            &current_freezes(),
            false,
            false,
        ),
        JudgeOutcome::Advice { .. }
    ));

    let mut complete_but_inconclusive = judge_result();
    complete_but_inconclusive["role_output"]["recommended_disposition"] = json!("inconclusive");
    assert_eq!(
        consume_judge(
            &task,
            "completed",
            Some(&complete_but_inconclusive),
            &current_freezes(),
            false,
            false,
        ),
        JudgeOutcome::Inconclusive("judge_declined_determinate_disposition")
    );

    let mut invalid_status = judge_result();
    invalid_status["role_output"]["answers"][0]["coverage_status"] = json!("complete material");
    assert_eq!(
        consume_judge(
            &task,
            "completed",
            Some(&invalid_status),
            &current_freezes(),
            false,
            false,
        ),
        JudgeOutcome::Degraded("invalid_judge_material_references")
    );

    let mut mismatched_required_coverage = task.clone();
    mismatched_required_coverage["role_input"]["questions"][0]["required_coverage"] =
        json!("entire packet");
    assert_eq!(
        consume_judge(
            &mismatched_required_coverage,
            "completed",
            Some(&judge_result()),
            &current_freezes(),
            false,
            false,
        ),
        JudgeOutcome::Degraded("invalid_judge_material_references")
    );

    let mut invalid_finding = judge_result();
    invalid_finding["role_output"]["findings"] = json!([{
        "severity": "major",
        "finding": "a referenced counterexample",
        "criterion_ids": ["JC1"],
        "material_ids": ["JM404"],
        "evidence": ["JM404"]
    }]);
    assert_eq!(
        consume_judge(
            &task,
            "completed",
            Some(&invalid_finding),
            &current_freezes(),
            false,
            false,
        ),
        JudgeOutcome::Degraded("invalid_judge_advisory_references")
    );

    let mut invalid_recommendation = judge_result();
    invalid_recommendation["role_output"]["recommendations"][0]["evidence"] = json!(["JM404"]);
    assert_eq!(
        consume_judge(
            &task,
            "completed",
            Some(&invalid_recommendation),
            &current_freezes(),
            false,
            false,
        ),
        JudgeOutcome::Degraded("invalid_judge_advisory_references")
    );

    let mut empty_questions = task.clone();
    empty_questions["role_input"]["questions"] = json!([]);
    assert_eq!(
        consume_judge(
            &empty_questions,
            "completed",
            Some(&judge_result()),
            &current_freezes(),
            false,
            false,
        ),
        JudgeOutcome::Degraded("invalid_judge_role_envelope")
    );
}

#[test]
fn budget_and_invalid_envelope_fail_closed() {
    let task = judge_task();
    let result = judge_result();
    let mut tiny_budget_task = task.clone();
    tiny_budget_task["output_format"]["result_budget"]["max_public_result_bytes"] = json!(1);
    assert_eq!(
        consume_judge(
            &tiny_budget_task,
            "completed",
            Some(&result),
            &current_freezes(),
            false,
            false,
        ),
        JudgeOutcome::Degraded("result_budget_exceeded")
    );

    let mut invalid = result;
    invalid.as_object_mut().unwrap().remove("overflow");
    assert_eq!(
        consume_judge(
            &task,
            "completed",
            Some(&invalid),
            &current_freezes(),
            false,
            false,
        ),
        JudgeOutcome::Degraded("invalid_result_envelope")
    );

    let mut invalid_status = judge_result();
    invalid_status["objective_results"][0]["status"] = json!("looks_good");
    assert_eq!(
        consume_judge(
            &task,
            "completed",
            Some(&invalid_status),
            &current_freezes(),
            false,
            false,
        ),
        JudgeOutcome::Degraded("invalid_result_envelope")
    );

    let mut dangling_evidence = judge_result();
    dangling_evidence["objective_results"][0]["evidence"] = json!(["JM404"]);
    assert_eq!(
        consume_judge(
            &task,
            "completed",
            Some(&dangling_evidence),
            &current_freezes(),
            false,
            false,
        ),
        JudgeOutcome::Degraded("invalid_result_evidence_references")
    );
}
