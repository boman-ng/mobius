# Role profiles

Use one profile per delegation. The main agent must combine the selected input and output with the common task and result envelope in `SKILL.md`, then inline the complete combination for the worker.

## Contents

- [Scout](#scout)
- [Researcher](#researcher)
- [Driver](#driver)
- [Verifier](#verifier)
- [Judge](#judge)

## Scout

Use Scout for read-only inspection of locally available facts.

Input:

```json
{
  "roots": [
    {"id": "SR1", "locator": "local entry point", "purpose": "why to inspect it"}
  ],
  "inspection_requests": [
    {
      "id": "SI1",
      "request": "what to locate, inventory, trace, or compare",
      "hints": ["symbols, files, or search terms"],
      "evidence_focus": ["implementation, test, configuration, or log evidence"]
    }
  ],
  "baselines": [
    {"id": "SB1", "locator": "optional comparison baseline", "purpose": "comparison purpose"}
  ]
}
```

Output in `role_output`:

```json
{
  "root_results": [
    {
      "root_id": "SR1",
      "status": "inspected | no_finding | inaccessible | partial",
      "coverage": "actual inspection scope",
      "evidence": []
    }
  ],
  "inspection_results": [
    {
      "request_id": "SI1",
      "status": "answered | no_finding | inaccessible | partial",
      "answer": "result",
      "evidence": []
    }
  ],
  "facts": [
    {"id": "SF1", "claim": "direct observation", "root_ids": ["SR1"], "evidence": []}
  ],
  "inferences": [
    {"claim": "inference", "basis_fact_ids": ["SF1"], "limits": []}
  ],
  "conflicts": [
    {"subject": "conflicting local observations", "evidence": []}
  ]
}
```

Require non-empty `roots` and `inspection_requests`. Return one coverage result per root and request, including no finding, inaccessible, and partial outcomes. Keep facts directly observable; make every inference cite its fact basis. Roots are starting points, not path allowlists.

## Researcher

Use Researcher for open-world questions requiring current, authoritative sources.

Input:

```json
{
  "questions": [
    {"id": "RQ1", "question": "open-world question"}
  ],
  "source_requirements": {
    "preferred_types": ["official documentation, standards, or original research"],
    "freshness": "target date, version, or freshness window",
    "authority_requirements": ["required authority signals"]
  },
  "starting_points": [
    {"id": "RS1", "locator": "known source or search entry", "purpose": "why to start here"}
  ],
  "comparison_dimensions": ["version differences, source conflicts, or option dimensions"]
}
```

Output in `role_output`:

```json
{
  "sources": [
    {
      "id": "SRC1",
      "title": "source title",
      "locator": "checkable source location",
      "publisher": "publisher",
      "published_or_version": "publication date or version",
      "accessed_at": "access date",
      "provenance": "primary | secondary | unknown",
      "authority_signals": [
        "official | standards_body | peer_reviewed | vendor | community | unknown"
      ]
    }
  ],
  "answers": [
    {
      "question_id": "RQ1",
      "answer": "answer or inability to determine",
      "assessment": "direct | indirect | disputed | unknown",
      "source_ids": ["SRC1"],
      "evidence": ["specific section, page, or paragraph"],
      "limits": []
    }
  ],
  "inferences": [
    {"claim": "cross-source inference", "source_ids": ["SRC1"], "limits": []}
  ],
  "source_conflicts": [
    {"subject": "conflict", "source_ids": ["SRC1"], "assessment": "effect on the answer"}
  ]
}
```

Require non-empty `questions`. Answer every question or mark it unknown. Prefer sources that meet the requested authority and freshness, preserve a locator for each source, and treat unsupported model memory as an assumption to check. Source count never substitutes for source quality.

## Driver

Use Driver to perform one bounded task with explicitly authorized effects through the host-native workflow.

Input:

```json
{
  "change_targets": [
    {
      "id": "DT1",
      "target": "file, component, data, or external object that may change",
      "requested_change": "expected change",
      "expected_outcome": "observable outcome"
    }
  ],
  "implementation_constraints": [
    {"id": "DC1", "constraint": "convention, reusable entry point, or relationship to preserve"}
  ],
  "validations": [
    {
      "id": "DV1",
      "check": "check, observation, or command",
      "expected": "pass condition",
      "target_ids": ["DT1"]
    }
  ]
}
```

Output in `role_output`:

```json
{
  "target_results": [
    {
      "target_id": "DT1",
      "status": "changed | unchanged | partial | failed",
      "result": "actual result",
      "effect_ids": ["E1"],
      "artifact_ids": ["A1"]
    }
  ],
  "commands": [
    {
      "id": "CMD1",
      "purpose": "why it ran",
      "command": "redacted command",
      "exit_code": 0,
      "result": "important output summary",
      "effect_ids": ["E1"],
      "validation_ids": ["DV1"]
    }
  ],
  "validation_results": [
    {
      "validation_id": "DV1",
      "status": "passed | failed | not_run | inconclusive",
      "actual": "observed result",
      "evidence": []
    }
  ],
  "deviations": [
    {"subject": "deviation", "reason": "why", "impact": "effect on the task"}
  ]
}
```

Require non-empty `change_targets` and `validations`. Return every validation result or a reason it was not run. Use objectives, change targets, forbidden boundaries, user authorization, and Runtime permissions to select ordinary actions; also match a supplied high-risk positive allowlist when applicable.

Record every attempted or actual side effect in the common `effects` inventory. Do not add a parallel `changes` field. Reference effect IDs from effectful commands and validation IDs from read-only validation commands. Redact every secret and credential.

## Verifier

Use Verifier to independently test claims without repairing the subject under review.

Input:

```json
{
  "subjects": [
    {"id": "VS1", "subject": "file, artifact, interface, or behavior to verify"}
  ],
  "claims": [
    {"id": "VC1", "claim": "claim to support, contradict, or leave unresolved"}
  ],
  "checks": [
    {
      "id": "VK1",
      "check": "method, observation, or command",
      "subject_ids": ["VS1"],
      "claim_ids": ["VC1"],
      "expected": "expected behavior or baseline",
      "counterexample": "signal that would refute it"
    }
  ],
  "environment": [
    {"id": "VE1", "condition": "version, platform, fixture, or prerequisite", "required": true}
  ]
}
```

Output in `role_output`:

```json
{
  "subject_results": [
    {
      "subject_id": "VS1",
      "status": "verified | contradicted | inconclusive | inaccessible | not_run",
      "evidence": []
    }
  ],
  "claim_results": [
    {
      "claim_id": "VC1",
      "assessment": "supports | contradicts | inconclusive | unknown | not_run",
      "evidence": []
    }
  ],
  "check_results": [
    {
      "check_id": "VK1",
      "status": "passed | failed | not_run | inconclusive",
      "actual": "observed result",
      "environment_ids": ["VE1"],
      "evidence": []
    }
  ],
  "discrepancies": [
    {"subject_id": "VS1", "expected": "expected", "actual": "actual", "impact": "impact", "evidence": []}
  ],
  "gaps": [
    {"subject": "verification gap", "reason": "why", "needed": "what would close it"}
  ]
}
```

Require non-empty `subjects` and at least one non-empty collection among `claims` and `checks`. Return every subject, claim, and check result. Do not repair the subject. Authorize and report any temporary test effects through the common effect contract; do not invent valueless commands merely to populate the template.

## Judge

Use Judge to challenge frozen material and return advice. Judge never owns later acceptance or submission.

Input:

```json
{
  "materials": [
    {
      "id": "JM1",
      "locator": "accessible material or inline content",
      "purpose": "why it is included",
      "freeze": {
        "method": "inline | content_digest | immutable_version | immutable_object_id",
        "value": "fixed content, digest, version, or stable object identity"
      }
    }
  ],
  "questions": [
    {
      "id": "JQ1",
      "question": "independent review question",
      "material_ids": ["JM1"],
      "required_coverage": "scope needed to answer"
    }
  ],
  "criteria": [
    {
      "id": "JC1",
      "criterion": "evaluation condition the Judge must not rewrite",
      "material_ids": ["JM1"],
      "required_coverage": "scope needed to assess it"
    }
  ],
  "known_risks": [
    {
      "id": "JR1",
      "risk": "specific concern to challenge",
      "material_ids": ["JM1"],
      "required_coverage": "scope needed to assess it"
    }
  ],
  "disposition_options": ["permitted advisory options"]
}
```

Output in `role_output`:

```json
{
  "material_results": [
    {
      "material_id": "JM1",
      "status": "reviewed | stale | unverifiable | inaccessible | partial",
      "freeze_check": {
        "status": "matched | mismatched | unverifiable",
        "observed": "observed content marker or reason it could not be checked"
      },
      "coverage": "actual review scope",
      "evidence": []
    }
  ],
  "answers": [
    {
      "question_id": "JQ1",
      "assessment": "answered | inconclusive",
      "answer": "answer or inability to determine",
      "material_ids": ["JM1"],
      "coverage_status": "complete | partial | unverifiable",
      "evidence": []
    }
  ],
  "criterion_assessments": [
    {
      "criterion_id": "JC1",
      "assessment": "satisfied | unsatisfied | inconclusive",
      "material_ids": ["JM1"],
      "coverage_status": "complete | partial | unverifiable",
      "evidence": [],
      "reason": "basis"
    }
  ],
  "risk_assessments": [
    {
      "risk_id": "JR1",
      "assessment": "observed | mitigated | unsupported | inconclusive",
      "material_ids": ["JM1"],
      "coverage_status": "complete | partial | unverifiable",
      "evidence": []
    }
  ],
  "findings": [
    {
      "severity": "minor | major | blocking",
      "finding": "problem or counterexample",
      "criterion_ids": ["JC1"],
      "material_ids": ["JM1"],
      "evidence": []
    }
  ],
  "recommended_disposition": "one supplied option, or inconclusive",
  "recommendations": [
    {"recommendation": "advice", "reason": "why", "evidence": []}
  ]
}
```

Require non-empty `materials`, `questions`, and `criteria`. Each question, criterion, and supplied known risk must identify all necessary material IDs and required coverage. Every material must carry one freeze declaration; a locator alone is not a freeze mechanism. Check each freeze before substantive review and never replace a stale task version with current live content.

Map a mismatched freeze to material status `stale`; map a freeze that cannot be checked to `unverifiable`. An inaccessible material remains `inaccessible`. None of these materials may support a determinate assessment.

Apply this gate without exception:

| Necessary material state | Coverage status | Question, criterion, and risk assessment | Overall recommended disposition |
|---|---|---|---|
| Freeze matched and required coverage complete | `complete` | A determinate assessment is permitted | One supplied option, or `inconclusive` |
| Freeze matched but required coverage partial | `partial` | `inconclusive` | `inconclusive` |
| Freeze mismatched or stale | `unverifiable` | `inconclusive` | `inconclusive` |
| Freeze unverifiable or material inaccessible | `unverifiable` | `inconclusive` | `inconclusive` |

Only matched materials with complete required coverage may support a determinate answer or assessment. Auxiliary background cannot expand the evidence set. Findings and recommendations cannot bypass the gate. If any question, criterion, or known risk is inconclusive because necessary material is incomplete, make the overall disposition `inconclusive`. Keep all outputs advisory.

Treat every listed status, assessment, and severity alternative as a closed enum. Close every
material, question, criterion, supplied risk, and corresponding result by one unique task-local ID.
Every finding criterion/material/evidence reference and every recommendation or common-envelope
artifact/evidence reference must resolve to the current task-local inventory. Missing, extra,
duplicate, unknown, or cross-inventory references make the result invalid; prose or disposition
cannot repair them.
