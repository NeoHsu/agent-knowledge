//! Durable semantic-edge validation and shared persistence helpers.

use std::collections::{HashMap, HashSet};

use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::db::{
    add_ambiguity_record, column_exists, memory_by_id, memory_by_name, new_event_uid,
    resolve_ambiguity_record, with_transaction,
};
use crate::error;
use crate::util::{normalize_rfc3339, now, sanitize_secret_field, source_priority, strip_secrets};

use super::ids::{artifact_node_id, memory_node_id, stable_hash_hex, workflow_step_node_id};
use super::model::*;
use super::store::{
    insert_edge, insert_simple_node, node_by_id, parse_json_value, relation_weight,
    set_graph_dirty, table_exists,
};
use super::{GRAPH_SCHEMA_VERSION, SEMANTIC, SEMANTIC_RELATIONS};

mod ingest;
mod merge;
mod projection;
mod review;

pub use ingest::ingest_semantic_edges;
use ingest::validate_concept_node_id;
pub use merge::merge_semantic_edges;
pub(super) use projection::materialize_semantic_edges;
pub use review::{review_semantic_edges, set_semantic_edge_status};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SemanticEdgePayload {
    #[serde(default = "default_graph_schema_version")]
    schema_version: i64,
    edges: Vec<SemanticEdgeInput>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct SemanticEdgeInput {
    id: Option<String>,
    source: String,
    target: String,
    relation: String,
    confidence: String,
    evidence: String,
    rationale: Option<String>,
    #[serde(default)]
    source_spans: Value,
    #[serde(default)]
    tags: Value,
    valid_until: Option<String>,
}

fn default_graph_schema_version() -> i64 {
    GRAPH_SCHEMA_VERSION
}

#[derive(Debug, Clone)]
struct ExistingSemanticEdge {
    id: String,
    source: String,
    status: String,
    ambiguity_id: Option<i64>,
    user_confirmed_at: Option<String>,
}

fn validate_semantic_edge_id(id: &str) -> std::result::Result<String, String> {
    let trimmed = id.trim();
    if trimmed.is_empty() {
        return Err("semantic edge id cannot be empty".to_string());
    }
    if trimmed.len() > 256 {
        return Err("semantic edge id cannot exceed 256 bytes".to_string());
    }
    if trimmed
        .chars()
        .any(|ch| ch.is_control() || ch.is_whitespace())
    {
        return Err(format!(
            "semantic edge id contains whitespace/control chars: {id}"
        ));
    }
    Ok(trimmed.to_string())
}

fn normalized_json_array(value: &Value) -> std::result::Result<Value, String> {
    if value.is_null() {
        return Ok(json!([]));
    }
    if !value.is_array() {
        return Err("source_spans must be an array".to_string());
    }
    Ok(strip_json_secrets(value))
}

fn normalized_string_array(value: &Value) -> std::result::Result<Value, String> {
    if value.is_null() {
        return Ok(json!([]));
    }
    let Some(array) = value.as_array() else {
        return Err("tags must be an array".to_string());
    };
    let mut tags = Vec::with_capacity(array.len());
    for item in array {
        let Some(tag) = item.as_str() else {
            return Err("tags must be an array of strings".to_string());
        };
        tags.push(strip_secrets(tag).map_err(|err| err.to_string())?);
    }
    Ok(json!(tags))
}

fn sanitize_json_secrets(value: &Value, field: &str, allow_redaction: bool) -> Result<Value> {
    let redacted = strip_json_secrets(value);
    if redacted != *value && !allow_redaction {
        return Err(error::safety_violation(format!(
            "secret-like value detected in {field}; merge rejected. \
             Remove the secret or pass --redact-secrets explicitly"
        )));
    }
    Ok(redacted)
}

fn strip_json_secrets(value: &Value) -> Value {
    match value {
        Value::String(value) => strip_secrets(value)
            .map(Value::String)
            .unwrap_or_else(|_| Value::String("[REDACTED]".to_string())),
        Value::Array(values) => Value::Array(values.iter().map(strip_json_secrets).collect()),
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(key, value)| (key.clone(), strip_json_secrets(value)))
                .collect(),
        ),
        other => other.clone(),
    }
}

fn semantic_edge_by_id(conn: &Connection, id: &str) -> Result<Option<ExistingSemanticEdge>> {
    conn.query_row(
        "SELECT id, source, status, ambiguity_id, user_confirmed_at
         FROM graph_semantic_edges WHERE id = ?1",
        params![id],
        |row| {
            Ok(ExistingSemanticEdge {
                id: row.get(0)?,
                source: row.get(1)?,
                status: row.get(2)?,
                ambiguity_id: row.get(3)?,
                user_confirmed_at: row.get(4)?,
            })
        },
    )
    .optional()
    .map_err(Into::into)
}

fn semantic_edge_by_logical_key(
    conn: &Connection,
    source_ref: &str,
    target_ref: &str,
    relation: &str,
) -> Result<Option<ExistingSemanticEdge>> {
    conn.query_row(
        "SELECT id, source, status, ambiguity_id, user_confirmed_at
         FROM graph_semantic_edges
         WHERE source_ref = ?1 AND target_ref = ?2 AND relation = ?3
           AND status IN ('active', 'pending')
         ORDER BY updated_at DESC, id
         LIMIT 1",
        params![source_ref, target_ref, relation],
        |row| {
            Ok(ExistingSemanticEdge {
                id: row.get(0)?,
                source: row.get(1)?,
                status: row.get(2)?,
                ambiguity_id: row.get(3)?,
                user_confirmed_at: row.get(4)?,
            })
        },
    )
    .optional()
    .map_err(Into::into)
}

#[allow(clippy::too_many_arguments)]
fn identical_semantic_edge_id(
    conn: &Connection,
    source_ref: &str,
    target_ref: &str,
    relation: &str,
    confidence: &str,
    status: &str,
    evidence: &str,
    valid_until: Option<&str>,
) -> Result<Option<String>> {
    conn.query_row(
        "SELECT id FROM graph_semantic_edges
         WHERE source_ref = ?1 AND target_ref = ?2 AND relation = ?3
           AND confidence = ?4 AND status = ?5 AND evidence = ?6
           AND COALESCE(valid_until, '') = COALESCE(?7, '')
         ORDER BY id LIMIT 1",
        params![
            source_ref,
            target_ref,
            relation,
            confidence,
            status,
            evidence,
            valid_until
        ],
        |row| row.get(0),
    )
    .optional()
    .map_err(Into::into)
}

fn log_semantic_edge_revision(conn: &Connection, edge_id: &str, action: &str) -> Result<()> {
    let uid = new_event_uid(conn, "semantic-revision")?;
    conn.execute(
        "INSERT INTO graph_semantic_edge_revisions
         (uid, edge_id, version, action, snapshot, source)
         SELECT ?3, id, version, ?2,
                json_object(
                    'source_ref', source_ref,
                    'target_ref', target_ref,
                    'relation', relation,
                    'confidence', confidence,
                    'status', status,
                    'evidence', evidence,
                    'rationale', rationale,
                    'source_spans', json(source_spans),
                    'tags', json(tags),
                    'generated_by', generated_by,
                    'source', source,
                    'valid_until', valid_until,
                    'ambiguity_id', ambiguity_id
                ),
                source
         FROM graph_semantic_edges
         WHERE id = ?1",
        params![edge_id, action, uid],
    )?;
    Ok(())
}

fn generated_by_for_source(source: &str) -> &'static str {
    if source == "manual" {
        "manual"
    } else {
        "agent"
    }
}

fn ensure_pending_edge_ambiguity(
    conn: &Connection,
    input: &SemanticEdgeInput,
    source_ref: &str,
    target_ref: &str,
    existing_ambiguity_id: Option<i64>,
    conflicts_with: Option<&str>,
    cross_scope: bool,
) -> Result<i64> {
    if let Some(id) = existing_ambiguity_id {
        return Ok(id);
    }
    let memory_ids = [source_ref, target_ref]
        .iter()
        .filter_map(|reference| reference.strip_prefix("memory:"))
        .map(str::to_string)
        .collect::<Vec<_>>();
    let context = json!({
        "source": source_ref,
        "target": target_ref,
        "relation": input.relation,
        "confidence": input.confidence,
        "evidence": input.evidence,
        "conflicts_with": conflicts_with,
        "cross_scope": cross_scope,
    })
    .to_string();
    add_ambiguity_record(
        conn,
        &format!("graph:{}", input.relation),
        &memory_ids,
        Some(&context),
    )
}

fn semantic_edge_crosses_project_scopes(
    conn: &Connection,
    source_ref: &str,
    target_ref: &str,
) -> Result<bool> {
    let source_scope = semantic_endpoint_scope(conn, source_ref)?;
    let target_scope = semantic_endpoint_scope(conn, target_ref)?;
    Ok(matches!(
        (source_scope.as_deref(), target_scope.as_deref()),
        (Some(source), Some(target))
            if source.starts_with("project:")
                && target.starts_with("project:")
                && source != target
    ))
}

fn semantic_endpoint_scope(conn: &Connection, reference: &str) -> Result<Option<String>> {
    if let Some(memory_ref) = reference.strip_prefix("memory:") {
        return Ok(memory_by_id(conn, memory_ref)?.map(|memory| memory.scope));
    }
    Ok(node_by_id(conn, reference)?.and_then(|node| node.scope))
}

fn semantic_ref_label(conn: &Connection, reference: &str) -> Result<Option<String>> {
    if let Some(memory_id) = reference.strip_prefix("memory:") {
        return Ok(memory_by_id(conn, memory_id)?.map(|memory| memory.name));
    }
    if let Some(node) = node_by_id(conn, reference)? {
        return Ok(Some(node.label));
    }
    Ok(reference
        .strip_prefix("concept:")
        .map(|label| label.to_string()))
}

fn load_semantic_edges(conn: &Connection) -> Result<Vec<GraphSemanticEdgeRow>> {
    let confirmed_expr = if column_exists(conn, "graph_semantic_edges", "user_confirmed_at")? {
        "user_confirmed_at"
    } else {
        "NULL AS user_confirmed_at"
    };
    let sql = format!(
        "SELECT id, source_ref, target_ref, relation, confidence, status, evidence,
                rationale, source_spans, tags, generated_by, source, {confirmed_expr}, created_at,
                updated_at, valid_until, ambiguity_id, version
         FROM graph_semantic_edges
         ORDER BY created_at, id"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([], row_to_semantic_edge)?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(Into::into)
}

fn endpoint_references_review_memory(reference: &str, review_memory_ids: &HashSet<String>) -> bool {
    let id = reference.strip_prefix("memory:").unwrap_or(reference);
    review_memory_ids.contains(id)
}

fn remap_merge_endpoint(
    conn: &Connection,
    reference: &str,
    memory_id_map: &HashMap<String, String>,
) -> Result<(String, bool)> {
    if let Some(memory_id) = reference.strip_prefix("memory:") {
        return Ok(match memory_id_map.get(memory_id) {
            Some(local_id) => (memory_node_id(local_id), true),
            None => (reference.to_string(), false),
        });
    }
    if let Some(step_ref) = reference.strip_prefix("workflow_step:") {
        if let Some((memory_id, step_id)) = step_ref.split_once(':') {
            return Ok(match memory_id_map.get(memory_id) {
                Some(local_id) => (workflow_step_node_id(local_id, step_id), true),
                None => (reference.to_string(), false),
            });
        }
        return Ok((reference.to_string(), false));
    }
    if reference.starts_with("concept:") {
        return validate_concept_node_id(reference)
            .map(|value| (value, true))
            .map_err(anyhow::Error::msg);
    }
    let recognized = [
        "artifact:",
        "tag:",
        "scope:",
        "type:",
        "source:",
        "claim:path:",
        "claim:command:",
    ]
    .iter()
    .any(|prefix| reference.starts_with(prefix));
    if recognized || node_by_id(conn, reference)?.is_some() {
        return Ok((reference.to_string(), true));
    }
    Ok((reference.to_string(), false))
}

#[allow(clippy::too_many_arguments)]
fn semantic_edge_matches(
    conn: &Connection,
    id: &str,
    source_ref: &str,
    target_ref: &str,
    relation: &str,
    confidence: &str,
    status: &str,
    evidence: &str,
    rationale: Option<&str>,
    source_spans: &Value,
    tags: &Value,
    source: &str,
    valid_until: Option<&str>,
) -> Result<bool> {
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM graph_semantic_edges
         WHERE id = ?1 AND source_ref = ?2 AND target_ref = ?3 AND relation = ?4
           AND confidence = ?5 AND status = ?6 AND evidence = ?7
           AND COALESCE(rationale, '') = COALESCE(?8, '')
           AND source_spans = ?9 AND tags = ?10 AND source = ?11
           AND COALESCE(valid_until, '') = COALESCE(?12, '')",
        params![
            id,
            source_ref,
            target_ref,
            relation,
            confidence,
            status,
            evidence,
            rationale,
            source_spans.to_string(),
            tags.to_string(),
            source,
            valid_until,
        ],
        |row| row.get(0),
    )?;
    Ok(count > 0)
}

#[allow(clippy::too_many_arguments)]
fn update_semantic_edge_from_merge(
    conn: &Connection,
    id: &str,
    source_ref: &str,
    target_ref: &str,
    incoming: &GraphSemanticEdgeRow,
    status: &str,
    evidence: &str,
    rationale: Option<&str>,
    source_spans: &Value,
    tags: &Value,
    ambiguity_id: Option<i64>,
) -> Result<()> {
    conn.execute(
        "UPDATE graph_semantic_edges
         SET source_ref = ?1, target_ref = ?2, relation = ?3, confidence = ?4,
             status = ?5, evidence = ?6, rationale = ?7, source_spans = ?8,
             tags = ?9, generated_by = ?10, source = ?11, user_confirmed_at = ?12,
             valid_until = ?13, ambiguity_id = ?14, updated_at = CURRENT_TIMESTAMP,
             version = version + 1
         WHERE id = ?15",
        params![
            source_ref,
            target_ref,
            incoming.relation,
            incoming.confidence,
            status,
            evidence,
            rationale,
            source_spans.to_string(),
            tags.to_string(),
            incoming.generated_by,
            incoming.source,
            incoming.user_confirmed_at,
            incoming.valid_until,
            ambiguity_id,
            id,
        ],
    )?;
    log_semantic_edge_revision(conn, id, "merge")?;
    Ok(())
}

fn unique_semantic_edge_id(conn: &Connection, preferred: &str) -> Result<String> {
    if semantic_edge_by_id(conn, preferred)?.is_none() {
        return Ok(preferred.to_string());
    }
    for suffix in 2..=10_000 {
        let candidate = format!("{preferred}_{suffix}");
        if semantic_edge_by_id(conn, &candidate)?.is_none() {
            return Ok(candidate);
        }
    }
    Err(error::conflict(format!(
        "could not allocate semantic edge id for {preferred}"
    )))
}

fn row_to_semantic_edge(row: &rusqlite::Row<'_>) -> rusqlite::Result<GraphSemanticEdgeRow> {
    let source_spans: String = row.get(8)?;
    let tags: String = row.get(9)?;
    Ok(GraphSemanticEdgeRow {
        id: row.get(0)?,
        source_ref: row.get(1)?,
        source_label: None,
        target_ref: row.get(2)?,
        target_label: None,
        relation: row.get(3)?,
        confidence: row.get(4)?,
        status: row.get(5)?,
        evidence: row.get(6)?,
        rationale: row.get(7)?,
        source_spans: parse_json_value(&source_spans),
        tags: parse_json_value(&tags),
        generated_by: row.get(10)?,
        source: row.get(11)?,
        user_confirmed_at: row.get(12)?,
        created_at: row.get(13)?,
        updated_at: row.get(14)?,
        valid_until: row.get(15)?,
        ambiguity_id: row.get(16)?,
        version: row.get(17)?,
    })
}
