//! Low-level graph persistence, row mapping, and metadata helpers.

use std::collections::BTreeMap;

use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};

use crate::util::now;

use super::model::{GraphEdge, GraphNode, GraphStats};
use super::{
    GRAPH_DIRTY_KEY, GRAPH_LAST_REBUILT_AT_KEY, GRAPH_SCHEMA_VERSION, GRAPH_SCHEMA_VERSION_KEY,
};

pub fn stats(conn: &Connection) -> Result<GraphStats> {
    Ok(GraphStats {
        status: "ok".to_string(),
        schema_version: metadata_value(conn, GRAPH_SCHEMA_VERSION_KEY)?
            .and_then(|value| value.parse::<i64>().ok())
            .unwrap_or(GRAPH_SCHEMA_VERSION),
        nodes: count_table(conn, "graph_nodes")?,
        edges: count_table(conn, "graph_edges")?,
        by_kind: grouped_count(conn, "graph_nodes", "kind")?,
        by_confidence: grouped_count(conn, "graph_edges", "confidence")?,
        by_status: grouped_count(conn, "graph_edges", "status")?,
        pending_edges: count_edges_by_status(conn, "pending")?,
        dirty: graph_dirty(conn)?,
        last_rebuilt_at: metadata_value(conn, GRAPH_LAST_REBUILT_AT_KEY)?,
    })
}

pub fn graph_dirty(conn: &Connection) -> Result<bool> {
    Ok(metadata_value(conn, GRAPH_DIRTY_KEY)?
        .map(|value| matches!(value.as_str(), "true" | "1"))
        .unwrap_or(false))
}

pub fn set_graph_dirty(conn: &Connection, dirty: bool) -> Result<()> {
    set_metadata(conn, GRAPH_DIRTY_KEY, if dirty { "true" } else { "false" })
}
pub(super) fn count_table(conn: &Connection, table: &str) -> Result<usize> {
    let sql = format!("SELECT COUNT(*) FROM {table}");
    let count: i64 = conn.query_row(&sql, [], |row| row.get(0))?;
    Ok(count.max(0) as usize)
}

pub(super) fn count_edges_by_origin(conn: &Connection, origin: &str) -> Result<usize> {
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM graph_edges WHERE origin = ?1",
        params![origin],
        |row| row.get(0),
    )?;
    Ok(count.max(0) as usize)
}

fn count_edges_by_status(conn: &Connection, status: &str) -> Result<usize> {
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM graph_edges WHERE status = ?1",
        params![status],
        |row| row.get(0),
    )?;
    Ok(count.max(0) as usize)
}

pub(super) fn query_json_rows_local(conn: &Connection, sql: &str) -> Result<Vec<Value>> {
    let mut stmt = conn.prepare(sql)?;
    let column_names = stmt
        .column_names()
        .iter()
        .map(|name| (*name).to_string())
        .collect::<Vec<_>>();
    let rows = stmt.query_map([], |row| {
        let mut object = serde_json::Map::new();
        for (index, name) in column_names.iter().enumerate() {
            let value: rusqlite::types::Value = row.get(index)?;
            object.insert(name.clone(), rusqlite_value_to_json(value));
        }
        Ok(Value::Object(object))
    })?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(Into::into)
}

fn rusqlite_value_to_json(value: rusqlite::types::Value) -> Value {
    match value {
        rusqlite::types::Value::Null => Value::Null,
        rusqlite::types::Value::Integer(value) => json!(value),
        rusqlite::types::Value::Real(value) => json!(value),
        rusqlite::types::Value::Text(value) => Value::String(value),
        rusqlite::types::Value::Blob(_) => Value::String("<blob>".to_string()),
    }
}

fn grouped_count(conn: &Connection, table: &str, column: &str) -> Result<BTreeMap<String, usize>> {
    let sql = format!("SELECT {column}, COUNT(*) FROM {table} GROUP BY {column} ORDER BY {column}");
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
    })?;
    let mut grouped = BTreeMap::new();
    for row in rows {
        let (key, count) = row?;
        grouped.insert(key, count.max(0) as usize);
    }
    Ok(grouped)
}

pub(super) fn metadata_value(conn: &Connection, key: &str) -> Result<Option<String>> {
    conn.query_row(
        "SELECT value FROM metadata WHERE key = ?1",
        params![key],
        |row| row.get(0),
    )
    .optional()
    .map_err(Into::into)
}

pub(super) fn set_metadata(conn: &Connection, key: &str, value: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO metadata (key, value, updated_at)
         VALUES (?1, ?2, ?3)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at
         WHERE metadata.value <> excluded.value",
        params![key, value, now()],
    )?;
    Ok(())
}
pub(super) fn parse_json_value(raw: &str) -> Value {
    serde_json::from_str(raw).unwrap_or_else(|_| json!(raw))
}

pub(super) fn table_exists(conn: &Connection, table: &str) -> Result<bool> {
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
        params![table],
        |row| row.get(0),
    )?;
    Ok(count > 0)
}

pub(super) fn insert_simple_node(
    conn: &Connection,
    id: &str,
    kind: &str,
    label: &str,
    scope: Option<&str>,
    origin: &str,
    metadata: Value,
) -> Result<()> {
    insert_node(
        conn,
        &GraphNode {
            id: id.to_string(),
            kind: kind.to_string(),
            label: label.to_string(),
            ref_table: None,
            ref_id: None,
            scope: scope.map(str::to_string),
            metadata,
            origin: origin.to_string(),
        },
    )
}

pub(super) fn insert_node(conn: &Connection, node: &GraphNode) -> Result<()> {
    let mut statement = conn.prepare_cached(
        "INSERT INTO graph_nodes
         (id, kind, label, ref_table, ref_id, scope, metadata, origin)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
         ON CONFLICT(id) DO UPDATE SET
            kind = excluded.kind,
            label = excluded.label,
            ref_table = COALESCE(excluded.ref_table, graph_nodes.ref_table),
            ref_id = COALESCE(excluded.ref_id, graph_nodes.ref_id),
            scope = COALESCE(excluded.scope, graph_nodes.scope),
            metadata = excluded.metadata,
            origin = excluded.origin,
            updated_at = CURRENT_TIMESTAMP",
    )?;
    statement.execute(params![
        node.id,
        node.kind,
        node.label,
        node.ref_table,
        node.ref_id,
        node.scope,
        node.metadata.to_string(),
        node.origin,
    ])?;
    Ok(())
}

pub(super) fn insert_edge(conn: &Connection, edge: &GraphEdge) -> Result<()> {
    let mut statement = conn.prepare_cached(
        "INSERT INTO graph_edges
         (id, source_node_id, target_node_id, relation, confidence, status, evidence, source_ref, scope, weight, origin, metadata)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
         ON CONFLICT(id) DO UPDATE SET
            source_node_id = excluded.source_node_id,
            target_node_id = excluded.target_node_id,
            relation = excluded.relation,
            confidence = excluded.confidence,
            status = excluded.status,
            evidence = excluded.evidence,
            source_ref = excluded.source_ref,
            scope = excluded.scope,
            weight = excluded.weight,
            origin = excluded.origin,
            metadata = excluded.metadata,
            updated_at = CURRENT_TIMESTAMP",
    )?;
    statement.execute(params![
        edge.id,
        edge.source_node_id,
        edge.target_node_id,
        edge.relation,
        edge.confidence,
        edge.status,
        edge.evidence,
        edge.source_ref,
        edge.scope,
        edge.weight,
        edge.origin,
        edge.metadata.to_string(),
    ])?;
    Ok(())
}

pub(super) fn relation_weight(relation: &str) -> f64 {
    match relation {
        "policy_for" | "risk_for" => 1.2,
        "depends_on" | "references_artifact" | "step_uses_artifact" => 1.1,
        "has_workflow_step"
        | "requires_confirmation"
        | "recorded_run"
        | "has_result"
        | "superseded_by" => 1.0,
        "mentions_path" | "mentions_command" | "same_theme" | "refines" | "contradicts"
        | "blocks" | "procedure_for" | "evidence_for" | "applies_to" | "mentions_concept" => 0.8,
        "has_tag" => 0.7,
        "related_to" => 0.5,
        "has_type" | "in_scope" | "from_source" => 0.2,
        _ => 0.6,
    }
}

pub(super) fn node_by_id(conn: &Connection, id: &str) -> Result<Option<GraphNode>> {
    conn.query_row(
        "SELECT id, kind, label, ref_table, ref_id, scope, metadata, origin
         FROM graph_nodes WHERE id = ?1",
        params![id],
        row_to_node,
    )
    .optional()
    .map_err(Into::into)
}

pub(super) fn row_to_node(row: &rusqlite::Row<'_>) -> rusqlite::Result<GraphNode> {
    let metadata: String = row.get(6)?;
    Ok(GraphNode {
        id: row.get(0)?,
        kind: row.get(1)?,
        label: row.get(2)?,
        ref_table: row.get(3)?,
        ref_id: row.get(4)?,
        scope: row.get(5)?,
        metadata: parse_json_value(&metadata),
        origin: row.get(7)?,
    })
}
