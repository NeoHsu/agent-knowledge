//! Materialized graph health and curation diagnostics.

use anyhow::Result;
use rusqlite::Connection;
use serde_json::{json, Value};

use super::store::{query_json_rows_local, stats};

pub fn graph_health(conn: &Connection) -> Result<Value> {
    let stats = stats(conn)?;
    let pending_edges: i64 = conn.query_row(
        "SELECT COUNT(*) FROM graph_semantic_edges
         WHERE status = 'pending'
           AND (valid_until IS NULL OR datetime(valid_until) >= datetime('now'))",
        [],
        |row| row.get(0),
    )?;
    let ambiguous_edges: i64 = conn.query_row(
        "SELECT COUNT(*) FROM graph_semantic_edges
         WHERE confidence = 'AMBIGUOUS' AND status IN ('active', 'pending')
           AND (valid_until IS NULL OR datetime(valid_until) >= datetime('now'))",
        [],
        |row| row.get(0),
    )?;
    if stats.dirty {
        return Ok(json!({
            "dirty": true,
            "derived_status": "stale",
            "nodes": stats.nodes,
            "edges": stats.edges,
            "pending_edges": pending_edges,
            "ambiguous_edges": ambiguous_edges,
            "dangling_semantic_edges": Value::Null,
            "orphan_memories": Value::Null,
            "old_pending_edges": Value::Null,
            "high_degree_nodes": Value::Null,
            "workflow_missing_safety_links": Value::Null,
            "artifact_blast_radius": Value::Null,
        }));
    }

    let dangling_semantic_edges = query_json_rows_local(
        conn,
        "SELECT id, source_ref, target_ref, relation FROM graph_semantic_edges
         WHERE status IN ('active', 'pending')
           AND (source_ref NOT IN (SELECT id FROM graph_nodes)
             OR target_ref NOT IN (SELECT id FROM graph_nodes))",
    )?;
    let orphan_memories = query_json_rows_local(
        conn,
        "SELECT n.id, n.label, n.scope FROM graph_nodes n
         WHERE n.kind = 'memory'
           AND COALESCE(json_extract(n.metadata, '$.lifecycle'), 'active') = 'active'
           AND NOT EXISTS (
             SELECT 1 FROM graph_edges e
             WHERE (e.source_node_id = n.id OR e.target_node_id = n.id)
               AND e.relation NOT IN ('has_type', 'in_scope', 'from_source')
           )
         ORDER BY n.id
         LIMIT 50",
    )?;
    let old_pending_edges = query_json_rows_local(
        conn,
        "SELECT id, source_ref, target_ref, relation, confidence, created_at
         FROM graph_semantic_edges
         WHERE status = 'pending'
           AND datetime(created_at) < datetime('now', '-30 days')
         ORDER BY created_at
         LIMIT 50",
    )?;
    let high_degree_nodes = query_json_rows_local(
        conn,
        "SELECT n.id, n.kind, n.label, COUNT(*) AS degree
         FROM graph_nodes n
         JOIN graph_edges e ON e.source_node_id = n.id OR e.target_node_id = n.id
         WHERE e.status = 'active'
           AND e.relation NOT IN ('has_type', 'in_scope', 'from_source')
         GROUP BY n.id, n.kind, n.label
         HAVING COUNT(*) >= 10
         ORDER BY degree DESC, n.id
         LIMIT 50",
    )?;
    let workflow_missing_safety_links = query_json_rows_local(
        conn,
        "SELECT DISTINCT workflow.id, workflow.label, workflow.scope
         FROM graph_nodes workflow
         JOIN graph_edges risk
           ON risk.source_node_id = workflow.id
          AND risk.target_node_id = 'tag:risk:high'
          AND risk.relation = 'has_tag'
         WHERE workflow.kind = 'memory'
           AND json_extract(workflow.metadata, '$.type') = 'workflow'
           AND NOT EXISTS (
             SELECT 1 FROM graph_edges policy
             WHERE (policy.source_node_id = workflow.id OR policy.target_node_id = workflow.id)
               AND policy.relation IN ('policy_for', 'risk_for')
               AND policy.status = 'active'
           )
         ORDER BY workflow.id
         LIMIT 50",
    )?;
    let artifact_blast_radius = query_json_rows_local(
        conn,
        "SELECT artifact.id, artifact.label, COUNT(DISTINCT edge.source_node_id) AS dependents
         FROM graph_nodes artifact
         JOIN graph_edges edge ON edge.target_node_id = artifact.id
         WHERE artifact.kind = 'artifact'
           AND edge.relation IN ('references_artifact', 'step_uses_artifact')
           AND edge.status = 'active'
         GROUP BY artifact.id, artifact.label
         ORDER BY dependents DESC, artifact.id
         LIMIT 50",
    )?;
    Ok(json!({
        "dirty": false,
        "derived_status": "current",
        "nodes": stats.nodes,
        "edges": stats.edges,
        "pending_edges": pending_edges,
        "ambiguous_edges": ambiguous_edges,
        "dangling_semantic_edges": dangling_semantic_edges,
        "orphan_memories": orphan_memories,
        "old_pending_edges": old_pending_edges,
        "high_degree_nodes": high_degree_nodes,
        "workflow_missing_safety_links": workflow_missing_safety_links,
        "artifact_blast_radius": artifact_blast_radius,
    }))
}
