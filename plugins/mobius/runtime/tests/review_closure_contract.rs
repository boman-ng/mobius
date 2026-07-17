use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use rusqlite::{Connection, params};
use serde_json::{Map, Value, json};
use sha2::{Digest as _, Sha256};

const OBJECTIVE: &str = "objective-review";
const LOOP_SKILL: &str = include_str!("../../skills/mobius-loop/SKILL.md");
const REVIEW_REFERENCE: &str = include_str!("../../skills/mobius-loop/references/review-read.md");

fn sha256_text(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let hex = digest
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!("sha256:{hex}")
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct Heads {
    project: u64,
    objective: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
enum Needed {
    Decision(String),
    Evidence(String),
    Packet(String),
}

fn exact_identity(kind: &str, id: &str) -> String {
    let mut identity = Map::new();
    identity.insert(kind.to_owned(), Value::String(id.to_owned()));
    Value::Object(identity).to_string()
}

fn insert_object(connection: &Connection, kind: &str, id: &str, value: Value) {
    connection
        .execute(
            "INSERT INTO object_projection
             (objective_id, object_kind, object_id, projection_bytes)
             VALUES (?1, ?2, ?3, ?4)",
            params![
                OBJECTIVE,
                kind,
                exact_identity(kind, id),
                value.to_string().as_bytes()
            ],
        )
        .expect("fixture object must insert");
}

fn heads(connection: &Connection) -> Heads {
    connection
        .query_row(
            "SELECT project_seq, objective_seq
             FROM schema_meta JOIN objective_streams ON objective_id = ?1
             WHERE singleton = 1",
            [OBJECTIVE],
            |row| {
                Ok(Heads {
                    project: row.get(0)?,
                    objective: row.get(1)?,
                })
            },
        )
        .expect("fixture heads must exist")
}

fn read_exact(
    connection: &Connection,
    kind: &str,
    id: &str,
    reads: &mut BTreeMap<(String, String), usize>,
) -> Value {
    let mut statement = connection
        .prepare(
            "SELECT projection_bytes FROM object_projection
             WHERE objective_id = ?1 AND object_kind = ?2 AND object_id = ?3",
        )
        .expect("exact read must prepare");
    let rows = statement
        .query_map(params![OBJECTIVE, kind, exact_identity(kind, id)], |row| {
            row.get::<_, Vec<u8>>(0)
        })
        .expect("exact read must execute")
        .collect::<Result<Vec<_>, _>>()
        .expect("exact rows must decode");
    assert_eq!(rows.len(), 1, "one declared identity must return one row");
    *reads.entry((kind.to_owned(), id.to_owned())).or_default() += 1;

    let object: Value = serde_json::from_slice(&rows[0]).expect("projection must be JSON");
    let body = object.get(kind).expect("projection kind must match");
    assert_eq!(body.get("id").and_then(Value::as_str), Some(id));
    body.clone()
}

fn verified_snapshot_range(
    project_root: &Path,
    snapshot: &Value,
    offset: u64,
    length: usize,
) -> Result<Vec<u8>, String> {
    let digest = snapshot
        .get("digest")
        .and_then(Value::as_str)
        .ok_or_else(|| "snapshot digest is missing".to_owned())?;
    let hex = digest
        .strip_prefix("sha256:")
        .filter(|hex| {
            hex.len() == 64
                && hex
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
        })
        .ok_or_else(|| "snapshot digest is not canonical".to_owned())?;
    let expected_size = snapshot
        .get("size_bytes")
        .and_then(Value::as_u64)
        .ok_or_else(|| "snapshot size is missing".to_owned())?;
    let canonical_root = fs::canonicalize(project_root).map_err(|error| error.to_string())?;
    let path = canonical_root.join(".mobius/artifacts/blobs").join(hex);

    let verify = || -> Result<(), String> {
        let metadata = fs::symlink_metadata(&path).map_err(|error| error.to_string())?;
        if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
            return Err("snapshot blob is not a regular non-symlink file".to_owned());
        }
        if fs::canonicalize(&path).map_err(|error| error.to_string())? != path {
            return Err("snapshot blob escaped its canonical locator".to_owned());
        }
        let mut file = File::open(&path).map_err(|error| error.to_string())?;
        let mut hasher = Sha256::new();
        let mut buffer = [0_u8; 64 * 1024];
        let mut actual_size = 0_u64;
        loop {
            let count = file.read(&mut buffer).map_err(|error| error.to_string())?;
            if count == 0 {
                break;
            }
            actual_size = actual_size
                .checked_add(count as u64)
                .ok_or_else(|| "snapshot size overflow".to_owned())?;
            hasher.update(&buffer[..count]);
        }
        let actual_digest = {
            let digest = hasher.finalize();
            let hex = digest
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>();
            format!("sha256:{hex}")
        };
        if actual_size != expected_size || actual_digest != digest {
            return Err("snapshot digest or size mismatch".to_owned());
        }
        Ok(())
    };

    verify()?;
    let length_u64 = u64::try_from(length).map_err(|error| error.to_string())?;
    let end = offset
        .checked_add(length_u64)
        .ok_or_else(|| "snapshot range overflow".to_owned())?;
    if end > expected_size {
        return Err("snapshot range exceeds the frozen size".to_owned());
    }
    let mut file = File::open(&path).map_err(|error| error.to_string())?;
    file.seek(SeekFrom::Start(offset))
        .map_err(|error| error.to_string())?;
    let mut bytes = Vec::with_capacity(length);
    file.take(length_u64)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() != length {
        return Err("snapshot range was truncated".to_owned());
    }
    verify()?;
    Ok(bytes)
}

#[test]
fn recursive_review_closure_includes_grandchild_and_deduplicates_convergence() {
    for contract in [
        "references/review-read.md",
        "Only after the live state is `Reviewing`",
        "A Decision is forbidden until exact row",
        "Do not load that recipe for\nother states",
    ] {
        assert!(LOOP_SKILL.contains(contract), "Loop omitted {contract}");
    }
    for contract in [
        "Recurse until no unseen dependency Decision remains",
        "Deduplicate each\nkind by immutable exact identity",
        "declared distinct identity count to equal the returned distinct row count",
        "Re-read both heads and the current\nPacket identity",
    ] {
        assert!(
            REVIEW_REFERENCE.contains(contract),
            "Review reference omitted {contract}"
        );
    }

    let connection = Connection::open_in_memory().expect("in-memory SQLite must open");
    connection
        .execute_batch(
            "CREATE TABLE schema_meta (
                 singleton INTEGER PRIMARY KEY,
                 project_seq INTEGER NOT NULL
             );
             CREATE TABLE objective_streams (
                 objective_id TEXT PRIMARY KEY,
                 objective_seq INTEGER NOT NULL
             );
             CREATE TABLE object_projection (
                 objective_id TEXT NOT NULL,
                 object_kind TEXT NOT NULL,
                 object_id TEXT NOT NULL,
                 projection_bytes BLOB NOT NULL,
                 PRIMARY KEY (objective_id, object_kind, object_id)
             );
             INSERT INTO schema_meta VALUES (1, 17);
             INSERT INTO objective_streams VALUES ('objective-review', 9);",
        )
        .expect("fixture schema must install");

    for (id, evidence, proofs) in [
        (
            "packet-root",
            "evidence-root",
            json!({"left": "decision-left", "right": "decision-right"}),
        ),
        (
            "packet-left",
            "evidence-left",
            json!({"shared": "decision-grandchild"}),
        ),
        (
            "packet-right",
            "evidence-right",
            json!({"shared": "decision-grandchild"}),
        ),
        ("packet-grandchild", "evidence-grandchild", json!({})),
    ] {
        insert_object(
            &connection,
            "review_packet",
            id,
            json!({"review_packet": {
                "id": id,
                "evidence_set": [evidence],
                "context": {"dependency_proofs": proofs}
            }}),
        );
        insert_object(
            &connection,
            "evidence",
            evidence,
            json!({"evidence": {"id": evidence}}),
        );
    }
    for (id, packet) in [
        ("decision-left", "packet-left"),
        ("decision-right", "packet-right"),
        ("decision-grandchild", "packet-grandchild"),
    ] {
        insert_object(
            &connection,
            "review_decision",
            id,
            json!({"review_decision": {"id": id, "packet": packet}}),
        );
    }

    let frozen_heads = heads(&connection);
    let mut pending = VecDeque::from([Needed::Packet("packet-root".to_owned())]);
    let mut seen = BTreeSet::new();
    let mut reads = BTreeMap::new();
    while let Some(needed) = pending.pop_front() {
        if !seen.insert(needed.clone()) {
            continue;
        }
        match needed {
            Needed::Packet(id) => {
                let packet = read_exact(&connection, "review_packet", &id, &mut reads);
                for evidence in packet["evidence_set"]
                    .as_array()
                    .expect("Evidence identities must be declared")
                {
                    pending.push_back(Needed::Evidence(
                        evidence
                            .as_str()
                            .expect("Evidence identity must be text")
                            .to_owned(),
                    ));
                }
                for decision in packet["context"]["dependency_proofs"]
                    .as_object()
                    .expect("dependency proofs must be declared")
                    .values()
                {
                    pending.push_back(Needed::Decision(
                        decision
                            .as_str()
                            .expect("Decision identity must be text")
                            .to_owned(),
                    ));
                }
            }
            Needed::Decision(id) => {
                let decision = read_exact(&connection, "review_decision", &id, &mut reads);
                pending.push_back(Needed::Packet(
                    decision["packet"]
                        .as_str()
                        .expect("Decision must declare its exact Packet")
                        .to_owned(),
                ));
            }
            Needed::Evidence(id) => {
                read_exact(&connection, "evidence", &id, &mut reads);
            }
        }
    }

    assert_eq!(heads(&connection), frozen_heads, "heads must be rechecked");
    for exact in [
        ("review_decision", "decision-grandchild"),
        ("review_packet", "packet-grandchild"),
        ("evidence", "evidence-grandchild"),
    ] {
        assert_eq!(
            reads.get(&(exact.0.to_owned(), exact.1.to_owned())),
            Some(&1)
        );
    }
    assert_eq!(
        reads.len(),
        11,
        "the closure must materialize each identity once"
    );
    assert!(reads.values().all(|count| *count == 1));
}

#[test]
fn core_snapshot_review_read_is_identity_bound_integrity_checked_and_bounded() {
    for contract in [
        ".mobius/artifacts/blobs/<digest-hex>",
        "64 lowercase hexadecimal characters",
        "full-file SHA-256",
        "[offset, offset + length)",
        "verification after the range read",
        "observational only",
    ] {
        assert!(
            REVIEW_REFERENCE.contains(contract),
            "Review reference omitted {contract}"
        );
    }

    let root =
        std::env::temp_dir().join(format!("mobius-review-snapshot-{}", uuid::Uuid::new_v4()));
    let blobs = root.join(".mobius/artifacts/blobs");
    fs::create_dir_all(&blobs).expect("fixture blob directory must exist");
    let root = root.canonicalize().expect("fixture root must canonicalize");
    let bytes = b"prefix:bounded-review-material:suffix";
    let digest = sha256_text(bytes);
    let hex = digest.strip_prefix("sha256:").unwrap();
    let blob = root.join(".mobius/artifacts/blobs").join(hex);
    fs::write(&blob, bytes).expect("fixture blob must write");

    let connection = Connection::open_in_memory().expect("in-memory SQLite must open");
    connection
        .execute_batch(
            "CREATE TABLE object_projection (
                 objective_id TEXT NOT NULL,
                 object_kind TEXT NOT NULL,
                 object_id TEXT NOT NULL,
                 projection_bytes BLOB NOT NULL,
                 PRIMARY KEY (objective_id, object_kind, object_id)
             );",
        )
        .expect("fixture schema must install");
    insert_object(
        &connection,
        "review_packet",
        "packet-snapshot",
        json!({"review_packet": {
            "id": "packet-snapshot",
            "evidence_set": ["evidence-snapshot"],
            "context": {"dependency_proofs": {}}
        }}),
    );
    insert_object(
        &connection,
        "evidence",
        "evidence-snapshot",
        json!({"evidence": {
            "id": "evidence-snapshot",
            "observation": {"core_snapshot": {
                "digest": digest,
                "size_bytes": bytes.len()
            }}
        }}),
    );

    let mut reads = BTreeMap::new();
    let packet = read_exact(&connection, "review_packet", "packet-snapshot", &mut reads);
    let evidence_id = packet["evidence_set"][0]
        .as_str()
        .expect("Packet must bind the exact Evidence identity");
    let evidence = read_exact(&connection, "evidence", evidence_id, &mut reads);
    let snapshot = &evidence["observation"]["core_snapshot"];
    assert_eq!(
        verified_snapshot_range(&root, snapshot, 7, 7).unwrap(),
        b"bounded"
    );
    assert_eq!(
        verified_snapshot_range(&root, snapshot, 7, bytes.len()).unwrap_err(),
        "snapshot range exceeds the frozen size"
    );

    let mut wrong_size = snapshot.clone();
    wrong_size["size_bytes"] = json!(bytes.len() + 1);
    assert_eq!(
        verified_snapshot_range(&root, &wrong_size, 7, 7).unwrap_err(),
        "snapshot digest or size mismatch"
    );
    fs::write(&blob, b"corrupt").expect("fixture corruption must write");
    assert_eq!(
        verified_snapshot_range(&root, snapshot, 0, 1).unwrap_err(),
        "snapshot digest or size mismatch"
    );

    fs::remove_dir_all(root).expect("fixture root must clean up");
}
