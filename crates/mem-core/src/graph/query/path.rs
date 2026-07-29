use std::collections::{HashMap, VecDeque};

use anyhow::{Context, Result};
use rusqlite::Connection;

use crate::error;

use super::super::model::{GraphEdge, GraphPathEdge, GraphPathOptions, GraphPathReport};
use super::super::store::node_by_id;
use super::resolve::{resolve_node_id, scope_allowed};
use super::traversal::{confidence_score, traversal_adjacency, traversal_edges};

pub fn shortest_path(
    conn: &Connection,
    from: &str,
    to: &str,
    options: GraphPathOptions,
) -> Result<GraphPathReport> {
    if options.max_depth > 20 {
        return Err(error::usage("graph path --max-depth cannot exceed 20"));
    }
    let from_id = resolve_node_id(conn, from, options.scope_filter.as_deref())?;
    let to_id = resolve_node_id(conn, to, options.scope_filter.as_deref())?;
    let from_node = node_by_id(conn, &from_id)?
        .ok_or_else(|| error::not_found(format!("node not found: {from_id}")))?;
    let to_node = node_by_id(conn, &to_id)?
        .ok_or_else(|| error::not_found(format!("node not found: {to_id}")))?;
    if !scope_allowed(from_node.scope.as_deref(), options.scope_filter.as_deref()) {
        return Err(error::not_found(format!(
            "graph node is outside the selected scope: {from_id}"
        )));
    }
    if !scope_allowed(to_node.scope.as_deref(), options.scope_filter.as_deref()) {
        return Err(error::not_found(format!(
            "graph node is outside the selected scope: {to_id}"
        )));
    }

    if from_id == to_id {
        return Ok(GraphPathReport {
            status: "ok".to_string(),
            from: from_node.clone(),
            to: to_node,
            hops: 0,
            path_score: 1.0,
            nodes: vec![from_node],
            edges: Vec::new(),
        });
    }

    let edges = traversal_edges(conn, options.clone())?;
    let adjacency = traversal_adjacency(&edges, options.direction, None);

    let mut queue = VecDeque::new();
    let mut best: HashMap<String, (usize, f64, String)> = HashMap::new();
    let mut previous: HashMap<String, (String, GraphEdge)> = HashMap::new();
    queue.push_back(from_id.clone());
    best.insert(from_id.clone(), (0, 1.0, from_id.clone()));

    while let Some(current) = queue.pop_front() {
        let Some((depth, path_score, path_key)) = best.get(&current).cloned() else {
            continue;
        };
        if depth >= options.max_depth {
            continue;
        }
        for (next, edge) in adjacency.get(&current).into_iter().flatten() {
            let next_depth = depth + 1;
            let next_score = path_score * edge.weight.max(0.0) * confidence_score(&edge.confidence);
            let next_key = format!("{path_key}\0{}\0{next}", edge.id);
            let should_update = match best.get(next) {
                None => true,
                Some((old_depth, old_score, old_key)) => {
                    next_depth < *old_depth
                        || (next_depth == *old_depth
                            && (next_score.total_cmp(old_score).is_gt()
                                || (next_score.total_cmp(old_score).is_eq()
                                    && next_key < *old_key)))
                }
            };
            if should_update {
                best.insert(next.clone(), (next_depth, next_score, next_key));
                previous.insert(next.clone(), (current.clone(), edge.clone()));
                queue.push_back(next.clone());
            }
        }
    }

    if !previous.contains_key(&to_id) {
        return Ok(GraphPathReport {
            status: "not_found".to_string(),
            from: from_node,
            to: to_node,
            hops: 0,
            path_score: 0.0,
            nodes: Vec::new(),
            edges: Vec::new(),
        });
    }

    let path_score = best
        .get(&to_id)
        .map(|(_, score, _)| *score)
        .unwrap_or_default();
    let mut node_ids = vec![to_id.clone()];
    let mut path_edges: Vec<GraphPathEdge> = Vec::new();
    let mut current = to_id.clone();
    while current != from_id {
        let (prev, edge) = previous
            .get(&current)
            .cloned()
            .with_context(|| format!("broken graph path at {current}"))?;
        path_edges.push(GraphPathEdge {
            id: edge.id,
            source: edge.source_node_id,
            target: edge.target_node_id,
            relation: edge.relation,
            confidence: edge.confidence,
            status: edge.status,
            evidence: edge.evidence,
            traversed_from: prev.clone(),
            traversed_to: current.clone(),
        });
        current = prev;
        node_ids.push(current.clone());
    }
    node_ids.reverse();
    path_edges.reverse();

    let mut nodes = Vec::with_capacity(node_ids.len());
    for id in &node_ids {
        if let Some(node) = node_by_id(conn, id)? {
            nodes.push(node);
        }
    }

    Ok(GraphPathReport {
        status: "ok".to_string(),
        from: from_node,
        to: to_node,
        hops: path_edges.len(),
        path_score,
        nodes,
        edges: path_edges,
    })
}
