use std::collections::{HashMap, HashSet};

use anyhow::Result;
use rusqlite::Connection;

use super::super::model::{GraphConfidenceFilter, GraphDirection, GraphEdge, GraphPathOptions};
use super::resolve::scope_allowed;
use super::row_to_edge;

pub(super) fn traversal_edges(
    conn: &Connection,
    options: GraphPathOptions,
) -> Result<Vec<GraphEdge>> {
    let mut stmt = conn.prepare(
        "SELECT id, source_node_id, target_node_id, relation, confidence, status,
                evidence, source_ref, scope, weight, origin, metadata
         FROM graph_edges
         WHERE status IN ('active', 'pending')",
    )?;
    let inactive_memories = inactive_memory_node_ids(conn)?;
    let scoped_nodes = scoped_node_ids(conn, options.scope_filter.as_deref())?;
    let rows = stmt.query_map([], row_to_edge)?;
    let mut edges = Vec::new();
    for row in rows {
        let edge = row?;
        if !options.include_metadata && is_metadata_relation(&edge.relation) {
            continue;
        }
        if let Some(scoped_nodes) = scoped_nodes.as_ref() {
            if !scoped_nodes.contains(&edge.source_node_id)
                || !scoped_nodes.contains(&edge.target_node_id)
            {
                continue;
            }
        }
        if edge.relation != "superseded_by"
            && (inactive_memories.contains(&edge.source_node_id)
                || inactive_memories.contains(&edge.target_node_id))
        {
            continue;
        }
        if edge.status == "pending"
            && !(options.include_ambiguous && edge.confidence == "AMBIGUOUS")
        {
            continue;
        }
        if !confidence_allowed(&edge.confidence, options.confidence)
            && !(options.include_ambiguous && edge.confidence == "AMBIGUOUS")
        {
            continue;
        }
        edges.push(edge);
    }
    Ok(edges)
}

pub(super) fn traversal_adjacency(
    edges: &[GraphEdge],
    direction: GraphDirection,
    per_node_limit: Option<usize>,
) -> HashMap<String, Vec<(String, GraphEdge)>> {
    let mut adjacency: HashMap<String, Vec<(String, GraphEdge)>> = HashMap::new();
    for edge in edges {
        if matches!(direction, GraphDirection::Any | GraphDirection::Outgoing) {
            adjacency
                .entry(edge.source_node_id.clone())
                .or_default()
                .push((edge.target_node_id.clone(), edge.clone()));
        }
        if matches!(direction, GraphDirection::Any | GraphDirection::Incoming)
            && (edge.source_node_id != edge.target_node_id
                || !matches!(direction, GraphDirection::Any))
        {
            adjacency
                .entry(edge.target_node_id.clone())
                .or_default()
                .push((edge.source_node_id.clone(), edge.clone()));
        }
    }
    for neighbors in adjacency.values_mut() {
        neighbors.sort_by(|left, right| {
            right
                .1
                .weight
                .total_cmp(&left.1.weight)
                .then_with(|| left.1.relation.cmp(&right.1.relation))
                .then_with(|| left.1.id.cmp(&right.1.id))
                .then_with(|| left.0.cmp(&right.0))
        });
        if let Some(limit) = per_node_limit {
            neighbors.truncate(limit);
        }
    }
    adjacency
}

fn scoped_node_ids(
    conn: &Connection,
    scope_filter: Option<&[String]>,
) -> Result<Option<HashSet<String>>> {
    let Some(scope_filter) = scope_filter else {
        return Ok(None);
    };
    let mut stmt = conn.prepare("SELECT id, scope FROM graph_nodes")?;
    let rows = stmt.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
    })?;
    let mut ids = HashSet::new();
    for row in rows {
        let (id, scope) = row?;
        if scope_allowed(scope.as_deref(), Some(scope_filter)) {
            ids.insert(id);
        }
    }
    Ok(Some(ids))
}

fn inactive_memory_node_ids(conn: &Connection) -> Result<HashSet<String>> {
    let mut stmt = conn.prepare(
        "SELECT id FROM graph_nodes
         WHERE kind = 'memory'
           AND json_extract(metadata, '$.lifecycle') = 'superseded'",
    )?;
    let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
    rows.collect::<rusqlite::Result<HashSet<_>>>()
        .map_err(Into::into)
}

fn is_metadata_relation(relation: &str) -> bool {
    matches!(relation, "has_type" | "in_scope" | "from_source")
}
fn confidence_allowed(confidence: &str, filter: GraphConfidenceFilter) -> bool {
    match filter {
        GraphConfidenceFilter::Extracted => confidence == "EXTRACTED",
        GraphConfidenceFilter::Inferred => matches!(confidence, "EXTRACTED" | "INFERRED"),
        GraphConfidenceFilter::All => true,
    }
}

pub(super) fn confidence_score(confidence: &str) -> f64 {
    match confidence {
        "EXTRACTED" => 1.0,
        "INFERRED" => 0.7,
        "AMBIGUOUS" => 0.3,
        _ => 0.1,
    }
}
