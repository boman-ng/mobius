use rusqlite::{Connection, params};

const WAIT_READ_REFERENCE: &str = include_str!("../../skills/mobius-loop/references/wait-read.md");
const OBJECTIVE_ID: &str = "objective-target";
const WAIT_ID: &str = "wait-target";
const WAIT_CONTEXT: &str = "release-signal";

#[derive(Clone, Debug, Eq, PartialEq)]
struct QueryRow {
    row_kind: String,
    matching_count: Option<i64>,
    payload_bytes: Option<i64>,
    within_budget: Option<i64>,
    object_id: Option<String>,
    payload: Option<String>,
}

#[derive(Debug, Eq, PartialEq)]
struct ValidatedOutput {
    admitted: bool,
    matching_count: usize,
    payload_bytes: usize,
    evidence: Vec<(String, String)>,
}

fn reference_query(max_evidence_items: usize, max_payload_bytes: usize) -> String {
    let fence = "```sql\n";
    let start = WAIT_READ_REFERENCE
        .find(fence)
        .expect("Wait reference must contain one SQL fence")
        + fence.len();
    let remainder = &WAIT_READ_REFERENCE[start..];
    let end = remainder
        .find("\n```")
        .expect("Wait reference SQL fence must be closed");
    assert!(
        !remainder[end + 4..].contains(fence),
        "Wait reference must have exactly one SQL fence"
    );

    let mut sql = remainder[..end].to_owned();
    for (placeholder, replacement) in [
        ("<objective-id>", OBJECTIVE_ID.to_owned()),
        ("<max-evidence-items>", max_evidence_items.to_string()),
        ("<max-payload-bytes>", max_payload_bytes.to_string()),
    ] {
        assert_eq!(
            sql.matches(placeholder).count(),
            1,
            "Wait reference must contain {placeholder} exactly once"
        );
        sql = sql.replace(placeholder, &replacement);
    }
    sql
}

fn database() -> Connection {
    let connection = Connection::open_in_memory().expect("open in-memory SQLite");
    connection
        .execute_batch(
            "CREATE TABLE objective_projection (
                 objective_id TEXT PRIMARY KEY,
                 projection_bytes BLOB NOT NULL
             ) STRICT;
             CREATE TABLE object_projection (
                 objective_id TEXT NOT NULL,
                 object_kind TEXT NOT NULL,
                 object_id TEXT NOT NULL,
                 projection_bytes BLOB NOT NULL,
                 PRIMARY KEY (objective_id, object_kind, object_id)
             ) STRICT;",
        )
        .expect("create projection tables");
    connection
}

fn seed_wait(connection: &Connection, objective_id: &str, wait_id: &str, context: &str) {
    let objective = format!(
        r#"{{"objective_state":{{"navigating":{{"navigation":{{"waiting":{{"wait_condition":"{wait_id}"}}}}}}}}}}"#
    );
    connection
        .execute(
            "INSERT INTO objective_projection (objective_id, projection_bytes) VALUES (?1, ?2)",
            params![objective_id, objective.as_bytes()],
        )
        .expect("insert Objective projection");

    let wait = format!(r#"{{"wait_condition":{{"id":"{wait_id}","context":"{context}"}}}}"#);
    insert_object(connection, objective_id, "wait_condition", wait_id, &wait);
}

fn evidence_payload(purpose: &str, wait_id: &str, context: &str, detail: &str) -> String {
    format!(
        r#"{{"evidence":{{"purpose":"{purpose}","subject":{{"wait_condition":"{wait_id}"}},"context":"{context}","detail":"{detail}"}}}}"#
    )
}

fn insert_object(
    connection: &Connection,
    objective_id: &str,
    object_kind: &str,
    object_id: &str,
    payload: &str,
) {
    connection
        .execute(
            "INSERT INTO object_projection (
                 objective_id, object_kind, object_id, projection_bytes
             ) VALUES (?1, ?2, ?3, ?4)",
            params![objective_id, object_kind, object_id, payload.as_bytes()],
        )
        .expect("insert object projection");
}

fn run_query(
    connection: &Connection,
    max_evidence_items: usize,
    max_payload_bytes: usize,
) -> Vec<QueryRow> {
    let sql = reference_query(max_evidence_items, max_payload_bytes);
    let mut statement = connection
        .prepare(&sql)
        .expect("prepare Wait reference SQL");
    statement
        .query_map([], |row| {
            Ok(QueryRow {
                row_kind: row.get(0)?,
                matching_count: row.get(1)?,
                payload_bytes: row.get(2)?,
                within_budget: row.get(3)?,
                object_id: row.get(4)?,
                payload: row.get(5)?,
            })
        })
        .expect("execute Wait reference SQL")
        .collect::<rusqlite::Result<Vec<_>>>()
        .expect("decode Wait reference rows")
}

fn validate_output(rows: &[QueryRow]) -> Result<ValidatedOutput, &'static str> {
    let summaries = rows
        .iter()
        .filter(|row| row.row_kind == "summary")
        .collect::<Vec<_>>();
    if summaries.len() != 1 {
        return Err("output must contain exactly one summary");
    }
    let summary = summaries[0];
    let (Some(matching_count), Some(payload_bytes), Some(within_budget)) = (
        summary.matching_count,
        summary.payload_bytes,
        summary.within_budget,
    ) else {
        return Err("summary is incomplete");
    };
    let (Ok(matching_count), Ok(payload_bytes)) = (
        usize::try_from(matching_count),
        usize::try_from(payload_bytes),
    ) else {
        return Err("summary counts must be non-negative");
    };
    let admitted = match within_budget {
        0 => false,
        1 => true,
        _ => return Err("summary admission must be boolean"),
    };

    let mut evidence = Vec::new();
    for row in rows.iter().filter(|row| row.row_kind != "summary") {
        if row.row_kind != "evidence"
            || row.matching_count.is_some()
            || row.payload_bytes.is_some()
            || row.within_budget.is_some()
        {
            return Err("evidence row is malformed");
        }
        let (Some(object_id), Some(payload)) = (&row.object_id, &row.payload) else {
            return Err("evidence row is incomplete");
        };
        evidence.push((object_id.clone(), payload.clone()));
    }

    if !admitted && !evidence.is_empty() {
        return Err("denied output must not contain evidence");
    }
    if admitted && evidence.len() != matching_count {
        return Err("admitted evidence count does not match summary");
    }
    if admitted
        && evidence
            .iter()
            .map(|(_, payload)| payload.len())
            .sum::<usize>()
            != payload_bytes
    {
        return Err("admitted payload bytes do not match summary");
    }

    Ok(ValidatedOutput {
        admitted,
        matching_count,
        payload_bytes,
        evidence,
    })
}

#[test]
fn reference_sql_admits_only_complete_matching_sets() {
    let empty = database();
    seed_wait(&empty, OBJECTIVE_ID, WAIT_ID, WAIT_CONTEXT);
    assert_eq!(
        validate_output(&run_query(&empty, 8, 4_096)),
        Ok(ValidatedOutput {
            admitted: true,
            matching_count: 0,
            payload_bytes: 0,
            evidence: Vec::new(),
        })
    );

    let admitted = database();
    seed_wait(&admitted, OBJECTIVE_ID, WAIT_ID, WAIT_CONTEXT);
    let first = evidence_payload("wait_resolution", WAIT_ID, WAIT_CONTEXT, "first");
    let second = evidence_payload("wait_resolution", WAIT_ID, WAIT_CONTEXT, "second");
    insert_object(&admitted, OBJECTIVE_ID, "evidence", "evidence-a", &first);
    insert_object(&admitted, OBJECTIVE_ID, "evidence", "evidence-b", &second);

    insert_object(
        &admitted,
        OBJECTIVE_ID,
        "evidence",
        "wrong-purpose",
        &evidence_payload("review", WAIT_ID, WAIT_CONTEXT, "excluded"),
    );
    insert_object(
        &admitted,
        OBJECTIVE_ID,
        "evidence",
        "wrong-wait",
        &evidence_payload("wait_resolution", "wait-other", WAIT_CONTEXT, "excluded"),
    );
    insert_object(
        &admitted,
        OBJECTIVE_ID,
        "evidence",
        "wrong-context",
        &evidence_payload("wait_resolution", WAIT_ID, "other-context", "excluded"),
    );
    insert_object(&admitted, OBJECTIVE_ID, "decision", "wrong-kind", &first);
    seed_wait(&admitted, "objective-other", WAIT_ID, WAIT_CONTEXT);
    insert_object(
        &admitted,
        "objective-other",
        "evidence",
        "wrong-objective",
        &first,
    );

    let expected_bytes = first.len() + second.len();
    let admitted_rows = run_query(&admitted, 2, expected_bytes);
    assert_eq!(
        validate_output(&admitted_rows),
        Ok(ValidatedOutput {
            admitted: true,
            matching_count: 2,
            payload_bytes: expected_bytes,
            evidence: vec![
                ("evidence-a".to_owned(), first.clone()),
                ("evidence-b".to_owned(), second.clone()),
            ],
        })
    );

    assert_eq!(
        validate_output(&run_query(&admitted, 1, 4_096)),
        Ok(ValidatedOutput {
            admitted: false,
            matching_count: 2,
            payload_bytes: expected_bytes,
            evidence: Vec::new(),
        })
    );

    let history = database();
    seed_wait(&history, OBJECTIVE_ID, WAIT_ID, WAIT_CONTEXT);
    let mut history_bytes = 0;
    for index in 0..512 {
        let payload = evidence_payload(
            "wait_resolution",
            WAIT_ID,
            WAIT_CONTEXT,
            &format!("{index:04}-{}", "x".repeat(256)),
        );
        history_bytes += payload.len();
        insert_object(
            &history,
            OBJECTIVE_ID,
            "evidence",
            &format!("history-{index:04}"),
            &payload,
        );
    }
    assert_eq!(
        validate_output(&run_query(&history, 512, history_bytes - 1)),
        Ok(ValidatedOutput {
            admitted: false,
            matching_count: 512,
            payload_bytes: history_bytes,
            evidence: Vec::new(),
        })
    );
}

#[test]
fn output_gate_rejects_partial_and_host_truncated_results() {
    let connection = database();
    seed_wait(&connection, OBJECTIVE_ID, WAIT_ID, WAIT_CONTEXT);
    for (id, detail) in [("evidence-a", "first"), ("evidence-b", "second")] {
        insert_object(
            &connection,
            OBJECTIVE_ID,
            "evidence",
            id,
            &evidence_payload("wait_resolution", WAIT_ID, WAIT_CONTEXT, detail),
        );
    }

    let rows = run_query(&connection, 2, 4_096);
    assert!(validate_output(&rows).is_ok());

    let mut partial = rows.clone();
    partial.pop();
    assert_eq!(
        validate_output(&partial),
        Err("admitted evidence count does not match summary")
    );

    let host_truncated_at_a_complete_row_boundary = rows[..1].to_vec();
    assert_eq!(
        validate_output(&host_truncated_at_a_complete_row_boundary),
        Err("admitted evidence count does not match summary")
    );
}
