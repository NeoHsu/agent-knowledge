use std::collections::{HashMap, HashSet};

use rusqlite::{Connection, params};
use serde_json::{Value, json};

use super::*;
use crate::graph::{GRAPH_SCHEMA_VERSION, GraphIngestOptions};

fn initialized_connection() -> Connection {
    let conn = Connection::open_in_memory().expect("open in-memory database");
    conn.execute_batch(include_str!("../../../../../schema/memory-schema.sql"))
        .expect("apply schema");
    crate::db::migrate_schema(&conn).expect("initialize metadata");
    conn
}

fn options(source: &str, user_confirmed: bool) -> GraphIngestOptions {
    GraphIngestOptions {
        pending_inferred: false,
        source: source.to_string(),
        user_confirmed,
        allow_secret_redaction: false,
    }
}

fn edge(
    id: Option<&str>,
    source: &str,
    target: &str,
    relation: &str,
    confidence: &str,
    evidence: &str,
) -> Value {
    json!({
        "id": id,
        "source": source,
        "target": target,
        "relation": relation,
        "confidence": confidence,
        "evidence": evidence,
        "rationale": null,
        "source_spans": [],
        "tags": [],
        "valid_until": null
    })
}

fn ingest_one(conn: &Connection, input: Value, options: GraphIngestOptions) -> GraphIngestReport {
    ingest_semantic_edges(
        conn,
        json!({"schema_version": GRAPH_SCHEMA_VERSION, "edges": [input]}),
        options,
    )
    .expect("ingest semantic edge")
}

#[test]
fn ingest_rejects_invalid_payloads_edges_and_unredacted_secrets() {
    let conn = initialized_connection();

    let version_error = ingest_semantic_edges(
        &conn,
        json!({"schema_version": GRAPH_SCHEMA_VERSION + 1, "edges": []}),
        options("agent", false),
    )
    .expect_err("unsupported schema version");
    assert!(
        version_error
            .to_string()
            .contains("unsupported semantic edge")
    );

    let too_many = vec![
        edge(
            None,
            "concept:a",
            "concept:b",
            "related_to",
            "EXTRACTED",
            "evidence",
        );
        1_001
    ];
    let size_error = ingest_semantic_edges(
        &conn,
        json!({"schema_version": GRAPH_SCHEMA_VERSION, "edges": too_many}),
        options("agent", false),
    )
    .expect_err("oversized payload");
    assert!(size_error.to_string().contains("cannot exceed 1000 edges"));

    let report = ingest_semantic_edges(
        &conn,
        json!({
            "schema_version": GRAPH_SCHEMA_VERSION,
            "edges": [
                edge(Some("bad_relation"), "concept:a", "concept:b", "owns", "EXTRACTED", "evidence"),
                edge(Some("bad_confidence"), "concept:a", "concept:b", "related_to", "CERTAIN", "evidence"),
                edge(Some("empty_evidence"), "concept:a", "concept:b", "related_to", "EXTRACTED", ""),
                edge(Some("unknown_endpoint"), "memory:missing", "concept:b", "related_to", "EXTRACTED", "evidence")
            ]
        }),
        options("agent", false),
    )
    .expect("reject invalid edges");
    assert_eq!(report.rejected, 4);
    assert!(
        report
            .results
            .iter()
            .all(|result| result.status == "rejected")
    );

    let manual = ingest_one(
        &conn,
        edge(
            Some("manual_without_confirmation"),
            "concept:a",
            "concept:b",
            "related_to",
            "EXTRACTED",
            "evidence",
        ),
        options("manual", false),
    );
    assert_eq!(manual.rejected, 1);
    assert!(
        manual.results[0]
            .reason
            .as_deref()
            .is_some_and(|reason| reason.contains("explicit user confirmation"))
    );

    let malformed_tags = json!({
        "schema_version": GRAPH_SCHEMA_VERSION,
        "edges": [{
            "id": "malformed_tags",
            "source": "concept:a",
            "target": "concept:b",
            "relation": "related_to",
            "confidence": "EXTRACTED",
            "evidence": "evidence",
            "source_spans": [],
            "tags": ["ok", 1]
        }]
    });
    assert!(
        ingest_semantic_edges(&conn, malformed_tags, options("agent", false))
            .expect_err("non-string tag")
            .to_string()
            .contains("array of strings")
    );

    let malformed_spans = json!({
        "schema_version": GRAPH_SCHEMA_VERSION,
        "edges": [{
            "id": "malformed_spans",
            "source": "concept:a",
            "target": "concept:b",
            "relation": "related_to",
            "confidence": "EXTRACTED",
            "evidence": "evidence",
            "source_spans": {"path": "README.md"},
            "tags": []
        }]
    });
    assert!(
        ingest_semantic_edges(&conn, malformed_spans, options("agent", false))
            .expect_err("non-array spans")
            .to_string()
            .contains("source_spans must be an array")
    );

    let secret = ["token", ": ", "abcdefgh", "12345678"].concat();
    let secret_input = edge(
        Some("secret_edge"),
        "concept:a",
        "concept:b",
        "related_to",
        "EXTRACTED",
        &secret,
    );
    assert!(
        ingest_semantic_edges(
            &conn,
            json!({"schema_version": GRAPH_SCHEMA_VERSION, "edges": [secret_input.clone()]}),
            options("agent", false),
        )
        .expect_err("secret must be rejected")
        .to_string()
        .contains("secret-like value")
    );
    let mut redacting = options("agent", false);
    redacting.allow_secret_redaction = true;
    let redacted = ingest_one(&conn, secret_input, redacting);
    assert_eq!(redacted.inserted, 1);
    let evidence: String = conn
        .query_row(
            "SELECT evidence FROM graph_semantic_edges WHERE id = 'secret_edge'",
            [],
            |row| row.get(0),
        )
        .expect("redacted evidence");
    assert_eq!(evidence, "[REDACTED]");
}

#[test]
fn ingest_handles_identity_trust_conflicts_and_pending_reclassification() {
    let conn = initialized_connection();
    let initial = edge(
        Some("edge_one"),
        "concept:a",
        "concept:b",
        "related_to",
        "EXTRACTED",
        "first evidence",
    );
    assert_eq!(
        ingest_one(&conn, initial.clone(), options("agent", false)).inserted,
        1
    );

    let duplicate = edge(
        None,
        "concept:a",
        "concept:b",
        "related_to",
        "EXTRACTED",
        "first evidence",
    );
    assert_eq!(
        ingest_one(&conn, duplicate, options("agent", false)).unchanged,
        1
    );

    let updated = edge(
        Some("edge_one"),
        "concept:a",
        "concept:b",
        "related_to",
        "EXTRACTED",
        "updated evidence",
    );
    assert_eq!(
        ingest_one(&conn, updated.clone(), options("agent", false)).updated,
        1
    );
    assert_eq!(
        ingest_one(&conn, updated.clone(), options("agent", false)).unchanged,
        1
    );
    assert_eq!(
        ingest_one(&conn, updated.clone(), options("daily_retro", false)).rejected,
        1
    );
    assert_eq!(
        ingest_one(&conn, updated.clone(), options("manual", true)).updated,
        1
    );

    let conflict = ingest_one(
        &conn,
        edge(
            Some("edge_conflict"),
            "concept:a",
            "concept:b",
            "related_to",
            "INFERRED",
            "competing evidence",
        ),
        options("manual", true),
    );
    assert_eq!(conflict.inserted, 1);
    assert_eq!(conflict.pending, 1);
    assert_eq!(conflict.results[0].edge_status.as_deref(), Some("pending"));
    assert_eq!(
        conflict.results[0].reason.as_deref(),
        Some("logical_edge_conflict_pending_review")
    );

    let ambiguous = edge(
        Some("edge_ambiguous"),
        "concept:c",
        "concept:d",
        "depends_on",
        "AMBIGUOUS",
        "uncertain evidence",
    );
    assert_eq!(
        ingest_one(&conn, ambiguous, options("manual", true)).pending,
        1
    );
    let reclassified = edge(
        Some("edge_ambiguous"),
        "concept:c",
        "concept:d",
        "depends_on",
        "EXTRACTED",
        "reviewed evidence",
    );
    let reclassified = ingest_one(&conn, reclassified, options("manual", true));
    assert_eq!(reclassified.updated, 1);
    assert_eq!(
        reclassified.results[0].edge_status.as_deref(),
        Some("active")
    );

    let mut inferred_options = options("agent", false);
    inferred_options.pending_inferred = true;
    let inferred = ingest_one(
        &conn,
        edge(
            Some("edge_inferred"),
            "concept:e",
            "concept:f",
            "evidence_for",
            "INFERRED",
            "inferred evidence",
        ),
        inferred_options,
    );
    assert_eq!(inferred.pending, 1);

    let resolved_ambiguities: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM ambiguities WHERE resolved_at IS NOT NULL",
            [],
            |row| row.get(0),
        )
        .expect("resolved ambiguities");
    assert!(resolved_ambiguities >= 1);
}

#[allow(clippy::too_many_arguments)]
fn insert_semantic_edge(
    conn: &Connection,
    id: &str,
    source_ref: &str,
    target_ref: &str,
    relation: &str,
    confidence: &str,
    status: &str,
    evidence: &str,
    source: &str,
    user_confirmed_at: Option<&str>,
) {
    let generated_by = if source == "manual" {
        "manual"
    } else {
        "agent"
    };
    conn.execute(
        "INSERT INTO graph_semantic_edges
         (id, source_ref, target_ref, relation, confidence, status, evidence,
          source_spans, tags, generated_by, source, user_confirmed_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, '[]', '[]', ?8, ?9, ?10)",
        params![
            id,
            source_ref,
            target_ref,
            relation,
            confidence,
            status,
            evidence,
            generated_by,
            source,
            user_confirmed_at,
        ],
    )
    .expect("insert semantic edge");
}

fn merge(
    conn: &Connection,
    incoming: &Connection,
    memory_id_map: &HashMap<String, String>,
    review_memory_ids: &HashSet<String>,
    prefer_trusted: bool,
    allow_secret_redaction: bool,
) -> GraphSemanticMergeReport {
    merge_semantic_edges(
        conn,
        incoming,
        memory_id_map,
        &HashMap::new(),
        review_memory_ids,
        prefer_trusted,
        allow_secret_redaction,
    )
    .expect("merge semantic edges")
}

#[test]
fn merge_imports_identical_edges_and_applies_trust_precedence() {
    let target = initialized_connection();
    let no_graph_table = Connection::open_in_memory().expect("open bare database");
    assert!(
        !merge(
            &target,
            &no_graph_table,
            &HashMap::new(),
            &HashSet::new(),
            true,
            false,
        )
        .changed()
    );

    let incoming = initialized_connection();
    insert_semantic_edge(
        &incoming,
        "merge_edge",
        "concept:a",
        "concept:b",
        "related_to",
        "EXTRACTED",
        "active",
        "agent evidence",
        "agent",
        None,
    );
    let imported = merge(
        &target,
        &incoming,
        &HashMap::new(),
        &HashSet::new(),
        true,
        false,
    );
    assert_eq!(imported.imported, 1);
    assert!(imported.changed());

    let identical = merge(
        &target,
        &incoming,
        &HashMap::new(),
        &HashSet::new(),
        true,
        false,
    );
    assert_eq!(identical.identical, 1);
    assert!(!identical.changed());

    incoming
        .execute(
            "UPDATE graph_semantic_edges
             SET evidence = 'trusted evidence', source = 'manual', generated_by = 'manual',
                 user_confirmed_at = '2026-08-04T00:00:00Z'
             WHERE id = 'merge_edge'",
            [],
        )
        .expect("upgrade incoming edge");
    let trusted = merge(
        &target,
        &incoming,
        &HashMap::new(),
        &HashSet::new(),
        true,
        false,
    );
    assert_eq!(trusted.trusted_updates, 1);

    incoming
        .execute(
            "UPDATE graph_semantic_edges
             SET evidence = 'lower trust', source = 'daily_retro', generated_by = 'agent',
                 user_confirmed_at = NULL
             WHERE id = 'merge_edge'",
            [],
        )
        .expect("downgrade incoming edge");
    let rejected = merge(
        &target,
        &incoming,
        &HashMap::new(),
        &HashSet::new(),
        true,
        false,
    );
    assert_eq!(rejected.rejected_lower_trust, 1);
}

#[test]
fn merge_marks_unresolved_conflicting_and_cross_scope_edges_pending() {
    let target = initialized_connection();
    let incoming = initialized_connection();
    insert_semantic_edge(
        &incoming,
        "unattested",
        "concept:u",
        "concept:v",
        "related_to",
        "EXTRACTED",
        "active",
        "manual claim",
        "manual",
        None,
    );
    insert_semantic_edge(
        &incoming,
        "unresolved",
        "memory:missing",
        "concept:v",
        "related_to",
        "EXTRACTED",
        "active",
        "missing endpoint",
        "agent",
        None,
    );
    insert_semantic_edge(
        &target,
        "local_conflict",
        "concept:x",
        "concept:y",
        "depends_on",
        "EXTRACTED",
        "active",
        "local evidence",
        "agent",
        None,
    );
    insert_semantic_edge(
        &incoming,
        "incoming_conflict",
        "concept:x",
        "concept:y",
        "depends_on",
        "INFERRED",
        "active",
        "incoming evidence",
        "agent",
        None,
    );

    target
        .execute_batch(
            "INSERT INTO memories
                 (id, type, name, content, tags, scope, source, confidence, protected,
                  created_at, updated_at)
             VALUES
                 ('local_a', 'reference', 'A', 'A', '[]', 'project:alpha', 'agent', 'medium', 0,
                  '2026-08-04T00:00:00Z', '2026-08-04T00:00:00Z'),
                 ('local_b', 'reference', 'B', 'B', '[]', 'project:beta', 'agent', 'medium', 0,
                  '2026-08-04T00:00:00Z', '2026-08-04T00:00:00Z');
             INSERT INTO graph_nodes
                 (id, kind, label, ref_table, ref_id, scope, metadata, origin)
             VALUES
                 ('memory:local_a', 'memory', 'A', 'memories', 'local_a', 'project:alpha', '{}', 'deterministic'),
                 ('memory:local_b', 'memory', 'B', 'memories', 'local_b', 'project:beta', '{}', 'deterministic');",
        )
        .expect("insert scoped memories and graph nodes");
    insert_semantic_edge(
        &incoming,
        "cross_scope",
        "memory:remote_a",
        "memory:remote_b",
        "depends_on",
        "EXTRACTED",
        "active",
        "cross-project evidence",
        "agent",
        None,
    );
    let memory_id_map = HashMap::from([
        ("remote_a".to_string(), "local_a".to_string()),
        ("remote_b".to_string(), "local_b".to_string()),
    ]);
    let review_memory_ids = HashSet::from(["missing".to_string()]);

    let report = merge(
        &target,
        &incoming,
        &memory_id_map,
        &review_memory_ids,
        false,
        false,
    );

    assert_eq!(report.unattested_manual_downgraded, 1);
    assert_eq!(report.unresolved_endpoints, 1);
    assert_eq!(report.conflicts, 1);
    assert_eq!(report.imported, 4);
    assert_eq!(report.pending, 3);
    let pending: i64 = target
        .query_row(
            "SELECT COUNT(*) FROM graph_semantic_edges WHERE status = 'pending'",
            [],
            |row| row.get(0),
        )
        .expect("pending edges");
    assert_eq!(pending, 3);
}

#[test]
fn merge_rejects_or_redacts_secrets_and_enforces_resource_limits() {
    let target = initialized_connection();
    let incoming = initialized_connection();
    let secret = ["token", ": ", "abcdefgh", "12345678"].concat();
    insert_semantic_edge(
        &incoming,
        "secret_merge",
        "concept:a",
        "concept:b",
        "related_to",
        "EXTRACTED",
        "active",
        &secret,
        "agent",
        None,
    );
    incoming
        .execute(
            "UPDATE graph_semantic_edges SET rationale = ?1, source_spans = ?2, tags = ?3
             WHERE id = 'secret_merge'",
            params![
                secret,
                json!([{"path": secret}]).to_string(),
                json!([secret]).to_string(),
            ],
        )
        .expect("add secret-bearing fields");

    let error = merge_semantic_edges(
        &target,
        &incoming,
        &HashMap::new(),
        &HashMap::new(),
        &HashSet::new(),
        true,
        false,
    )
    .expect_err("secret merge must fail closed");
    assert!(error.to_string().contains("secret-like value"));

    let redacted = merge(
        &target,
        &incoming,
        &HashMap::new(),
        &HashSet::new(),
        true,
        true,
    );
    assert_eq!(redacted.imported, 1);
    let fields: (String, String, String, String) = target
        .query_row(
            "SELECT evidence, rationale, source_spans, tags
             FROM graph_semantic_edges WHERE id = 'secret_merge'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .expect("redacted fields");
    for field in [&fields.0, &fields.1, &fields.2, &fields.3] {
        assert!(field.contains("[REDACTED]"));
        assert!(!field.contains("abcdefgh"));
    }

    let oversized_target = initialized_connection();
    let oversized_incoming = initialized_connection();
    insert_semantic_edge(
        &oversized_incoming,
        "oversized",
        "concept:a",
        "concept:b",
        "related_to",
        "EXTRACTED",
        "active",
        &"x".repeat(20_001),
        "agent",
        None,
    );
    let error = merge_semantic_edges(
        &oversized_target,
        &oversized_incoming,
        &HashMap::new(),
        &HashMap::new(),
        &HashSet::new(),
        true,
        false,
    )
    .expect_err("oversized evidence must fail");
    assert!(error.to_string().contains("exceeds 20000 characters"));
}
