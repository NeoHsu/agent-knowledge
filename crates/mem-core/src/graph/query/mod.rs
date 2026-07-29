//! Read-only graph explanation, path, export, candidate, and neighborhood operations.

use anyhow::Result;
use rusqlite::{params, Connection};

use crate::error;

use super::model::{GraphEdge, GraphExplain, GraphNeighbor};
use super::store::{node_by_id, parse_json_value};

mod candidates;
mod export;
mod neighborhood;
mod path;
mod resolve;
mod traversal;

pub use candidates::candidates;
pub use export::export_json;
pub use neighborhood::query_neighborhood;
pub use path::shortest_path;
pub use resolve::resolve_query_start_nodes;

use resolve::{resolve_node_id, scope_allowed};

pub fn explain(
    conn: &Connection,
    reference: &str,
    depth: usize,
    scope_filter: Option<&[String]>,
) -> Result<GraphExplain> {
    if depth > 1 {
        return Err(error::usage(
            "graph explain currently supports --depth 0 or 1",
        ));
    }
    let node_id = resolve_node_id(conn, reference, scope_filter)?;
    let node = node_by_id(conn, &node_id)?
        .ok_or_else(|| error::not_found(format!("node not found: {node_id}")))?;
    if !scope_allowed(node.scope.as_deref(), scope_filter) {
        return Err(error::not_found(format!(
            "graph node is outside the selected scope: {node_id}"
        )));
    }
    if depth == 0 {
        return Ok(GraphExplain {
            status: "ok".to_string(),
            node,
            neighbors: Vec::new(),
        });
    }

    let mut stmt = conn.prepare(
        "SELECT e.id, e.source_node_id, e.target_node_id, e.relation, e.confidence, e.status,
                e.evidence, e.source_ref, e.scope, e.weight, e.origin, e.metadata
         FROM graph_edges e
         WHERE e.status = 'active'
           AND (e.source_node_id = ?1 OR e.target_node_id = ?1)
         ORDER BY e.relation, e.target_node_id, e.source_node_id",
    )?;
    let rows = stmt.query_map(params![node_id], row_to_edge)?;
    let mut neighbors = Vec::new();
    for row in rows {
        let edge = row?;
        let (direction, other_id) = if edge.source_node_id == node_id {
            ("outgoing", edge.target_node_id.clone())
        } else {
            ("incoming", edge.source_node_id.clone())
        };
        if let Some(other) = node_by_id(conn, &other_id)? {
            if !scope_allowed(other.scope.as_deref(), scope_filter) {
                continue;
            }
            neighbors.push(GraphNeighbor {
                direction: direction.to_string(),
                relation: edge.relation,
                confidence: edge.confidence,
                status: edge.status,
                edge_id: edge.id,
                node: other,
                evidence: edge.evidence,
            });
        }
    }

    Ok(GraphExplain {
        status: "ok".to_string(),
        node,
        neighbors,
    })
}

pub(super) fn row_to_edge(row: &rusqlite::Row<'_>) -> rusqlite::Result<GraphEdge> {
    let metadata: String = row.get(11)?;
    Ok(GraphEdge {
        id: row.get(0)?,
        source_node_id: row.get(1)?,
        target_node_id: row.get(2)?,
        relation: row.get(3)?,
        confidence: row.get(4)?,
        status: row.get(5)?,
        evidence: row.get(6)?,
        source_ref: row.get(7)?,
        scope: row.get(8)?,
        weight: row.get(9)?,
        origin: row.get(10)?,
        metadata: parse_json_value(&metadata),
    })
}
