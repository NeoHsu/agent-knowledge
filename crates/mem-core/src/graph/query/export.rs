use anyhow::Result;
use rusqlite::Connection;

use super::super::GRAPH_SCHEMA_VERSION;
use super::super::model::{GraphEdge, GraphExport, GraphExportEdge, GraphExportNode, GraphNode};
use super::super::store::row_to_node;
use super::row_to_edge;

pub fn export_json(conn: &Connection) -> Result<GraphExport> {
    let nodes = all_nodes(conn)?
        .into_iter()
        .map(|node| GraphExportNode {
            id: node.id,
            label: node.label,
            kind: node.kind,
            metadata: node.metadata,
        })
        .collect();
    let edges = all_edges(conn)?
        .into_iter()
        .map(|edge| GraphExportEdge {
            source: edge.source_node_id,
            target: edge.target_node_id,
            relation: edge.relation,
            confidence: edge.confidence,
            status: edge.status,
            evidence: edge.evidence,
            metadata: edge.metadata,
        })
        .collect();
    Ok(GraphExport {
        schema_version: GRAPH_SCHEMA_VERSION,
        nodes,
        edges,
    })
}

fn all_nodes(conn: &Connection) -> Result<Vec<GraphNode>> {
    let mut stmt = conn.prepare(
        "SELECT id, kind, label, ref_table, ref_id, scope, metadata, origin
         FROM graph_nodes ORDER BY id",
    )?;
    let rows = stmt.query_map([], row_to_node)?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(Into::into)
}

fn all_edges(conn: &Connection) -> Result<Vec<GraphEdge>> {
    let mut stmt = conn.prepare(
        "SELECT id, source_node_id, target_node_id, relation, confidence, status,
                evidence, source_ref, scope, weight, origin, metadata
         FROM graph_edges ORDER BY source_node_id, relation, target_node_id",
    )?;
    let rows = stmt.query_map([], row_to_edge)?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(Into::into)
}
