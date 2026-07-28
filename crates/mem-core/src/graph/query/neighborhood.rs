use std::collections::{HashMap, HashSet, VecDeque};

use anyhow::{bail, Result};
use rusqlite::Connection;
use serde_json::Value;

use super::super::model::{
    GraphNode, GraphPathOptions, GraphQueryEdge, GraphQueryNode, GraphQueryOptions,
    GraphQueryReport, GraphQueryStart,
};
use super::super::store::node_by_id;
use super::resolve::scope_allowed;
use super::traversal::{confidence_score, traversal_adjacency, traversal_edges};

pub fn query_neighborhood(
    conn: &Connection,
    query: &str,
    starts: &[GraphQueryStart],
    options: GraphQueryOptions,
) -> Result<GraphQueryReport> {
    if options.depth > 8 {
        bail!("graph query --depth cannot exceed 8");
    }
    if options.limit == 0 || options.limit > 500 {
        bail!("graph query --limit must be between 1 and 500");
    }
    if starts.is_empty() {
        return Ok(GraphQueryReport {
            status: "no_start_nodes".to_string(),
            query: query.to_string(),
            start_nodes: Vec::new(),
            nodes: Vec::new(),
            edges: Vec::new(),
        });
    }

    let mut start_nodes = Vec::new();
    let mut start_scores = HashMap::new();
    for start in starts {
        if let Some(node) = node_by_id(conn, &start.node_id)? {
            if scope_allowed(node.scope.as_deref(), options.scope_filter.as_deref()) {
                start_scores.insert(node.id.clone(), start.score);
                start_nodes.push(node);
            }
        }
    }
    if start_nodes.is_empty() {
        return Ok(GraphQueryReport {
            status: "no_start_nodes".to_string(),
            query: query.to_string(),
            start_nodes,
            nodes: Vec::new(),
            edges: Vec::new(),
        });
    }

    let edges = traversal_edges(
        conn,
        GraphPathOptions {
            max_depth: options.depth,
            include_ambiguous: options.include_ambiguous,
            include_metadata: options.include_metadata,
            confidence: options.confidence,
            direction: options.direction,
            scope_filter: options.scope_filter.clone(),
        },
    )?;
    let adjacency = traversal_adjacency(&edges, options.direction, Some(100));

    let mut queue = VecDeque::new();
    let mut best: HashMap<String, (usize, f64, String)> = HashMap::new();
    for node in &start_nodes {
        let score = start_scores.get(&node.id).copied().unwrap_or(1.0);
        let should_update = best
            .get(&node.id)
            .map(|(_, old_score, _)| score.total_cmp(old_score).is_gt())
            .unwrap_or(true);
        if should_update {
            best.insert(node.id.clone(), (0, score, node.id.clone()));
            queue.push_back(node.id.clone());
        }
    }

    while let Some(current) = queue.pop_front() {
        let Some((depth, score, path_key)) = best.get(&current).cloned() else {
            continue;
        };
        if depth >= options.depth {
            continue;
        }
        for (next, edge) in adjacency.get(&current).into_iter().flatten() {
            let Some(next_node) = node_by_id(conn, next)? else {
                continue;
            };
            if !scope_allowed(next_node.scope.as_deref(), options.scope_filter.as_deref()) {
                continue;
            }
            let next_depth = depth + 1;
            let edge_score = score
                * edge.weight
                * confidence_score(&edge.confidence)
                * depth_score(next_depth)
                * node_quality_score(&next_node);
            let next_key = format!("{path_key}\0{}\0{next}", edge.id);
            let should_update = best
                .get(next)
                .map(|(old_depth, old_score, old_key)| {
                    next_depth < *old_depth
                        || (next_depth == *old_depth
                            && (edge_score.total_cmp(old_score).is_gt()
                                || (edge_score.total_cmp(old_score).is_eq()
                                    && next_key < *old_key)))
                })
                .unwrap_or(true);
            if should_update {
                best.insert(next.clone(), (next_depth, edge_score, next_key));
                queue.push_back(next.clone());
            }
        }
    }

    let mut nodes = Vec::new();
    for (node_id, (depth, score, _)) in best {
        if let Some(node) = node_by_id(conn, &node_id)? {
            nodes.push(GraphQueryNode { node, depth, score });
        }
    }
    nodes.sort_by(|left, right| {
        right
            .score
            .partial_cmp(&left.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| left.depth.cmp(&right.depth))
            .then_with(|| left.node.id.cmp(&right.node.id))
    });
    nodes.truncate(options.limit);
    let kept = nodes
        .iter()
        .map(|node| node.node.id.clone())
        .collect::<HashSet<_>>();
    let node_scores = nodes
        .iter()
        .map(|node| (node.node.id.clone(), node.score))
        .collect::<HashMap<_, _>>();
    let mut edges = edges
        .into_iter()
        .filter(|edge| kept.contains(&edge.source_node_id) && kept.contains(&edge.target_node_id))
        .map(|edge| {
            let endpoint_score = node_scores
                .get(&edge.source_node_id)
                .copied()
                .unwrap_or_default()
                .max(
                    node_scores
                        .get(&edge.target_node_id)
                        .copied()
                        .unwrap_or_default(),
                );
            GraphQueryEdge {
                id: edge.id,
                source: edge.source_node_id,
                target: edge.target_node_id,
                relation: edge.relation,
                confidence: edge.confidence.clone(),
                status: edge.status,
                evidence: edge.evidence,
                score: endpoint_score * edge.weight * confidence_score(&edge.confidence),
            }
        })
        .collect::<Vec<_>>();
    edges.sort_by(|left, right| {
        right
            .score
            .partial_cmp(&left.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| left.relation.cmp(&right.relation))
            .then_with(|| left.id.cmp(&right.id))
    });

    Ok(GraphQueryReport {
        status: "ok".to_string(),
        query: query.to_string(),
        start_nodes,
        nodes,
        edges,
    })
}
fn node_quality_score(node: &GraphNode) -> f64 {
    if node.kind != "memory" {
        return 1.0;
    }
    let source_score = match node.metadata.get("source").and_then(Value::as_str) {
        Some("manual") => 1.0,
        Some("agent") => 0.8,
        Some("daily_retro") => 0.6,
        Some("weekly_retro") => 0.5,
        _ => 0.7,
    };
    let confidence_score = match node.metadata.get("confidence").and_then(Value::as_str) {
        Some("high") => 1.0,
        Some("medium") => 0.8,
        Some("low") => 0.6,
        _ => 0.8,
    };
    source_score * confidence_score
}

fn depth_score(depth: usize) -> f64 {
    match depth {
        0 => 1.0,
        1 => 0.8,
        2 => 0.55,
        3 => 0.35,
        _ => 0.2,
    }
}
