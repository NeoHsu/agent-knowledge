//! Read-only graph explanation, path, export, candidate, and neighborhood operations.

use std::collections::{HashMap, HashSet, VecDeque};

use anyhow::{bail, Context, Result};
use rusqlite::{params, Connection};
use serde_json::Value;

use crate::db::{memory_by_id, memory_by_name, memory_by_name_in_scope};
use crate::util::parse_string_array;

use super::ids::memory_node_id;
use super::model::*;
use super::store::{node_by_id, parse_json_value, row_to_node};
use super::{GRAPH_SCHEMA_VERSION, SEMANTIC_RELATIONS};

pub fn explain(
    conn: &Connection,
    reference: &str,
    depth: usize,
    scope_filter: Option<&[String]>,
) -> Result<GraphExplain> {
    if depth > 1 {
        bail!("graph explain currently supports --depth 0 or 1");
    }
    let node_id = resolve_node_id(conn, reference, scope_filter)?;
    let node = node_by_id(conn, &node_id)?.with_context(|| format!("node not found: {node_id}"))?;
    if !scope_allowed(node.scope.as_deref(), scope_filter) {
        bail!("graph node is outside the selected scope: {node_id}");
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

pub fn shortest_path(
    conn: &Connection,
    from: &str,
    to: &str,
    options: GraphPathOptions,
) -> Result<GraphPathReport> {
    if options.max_depth > 20 {
        bail!("graph path --max-depth cannot exceed 20");
    }
    let from_id = resolve_node_id(conn, from, options.scope_filter.as_deref())?;
    let to_id = resolve_node_id(conn, to, options.scope_filter.as_deref())?;
    let from_node =
        node_by_id(conn, &from_id)?.with_context(|| format!("node not found: {from_id}"))?;
    let to_node = node_by_id(conn, &to_id)?.with_context(|| format!("node not found: {to_id}"))?;
    if !scope_allowed(from_node.scope.as_deref(), options.scope_filter.as_deref()) {
        bail!("graph node is outside the selected scope: {from_id}");
    }
    if !scope_allowed(to_node.scope.as_deref(), options.scope_filter.as_deref()) {
        bail!("graph node is outside the selected scope: {to_id}");
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

pub fn candidates(
    conn: &Connection,
    scope_filter: Option<&[&str]>,
    memory_type: Option<&str>,
    changed_since: Option<&str>,
    unlinked: bool,
    limit: usize,
) -> Result<GraphCandidates> {
    let mut memories =
        crate::db::list_memories_filtered(conn, false, memory_type, None, scope_filter, false)?;
    if let Some(changed_since) = changed_since {
        memories.retain(|memory| memory.updated_at.as_str() >= changed_since);
    }
    if unlinked {
        let mut unlinked_memories = Vec::new();
        for memory in memories {
            if !memory_has_useful_graph_edge(conn, &memory.id)? {
                unlinked_memories.push(memory);
            }
        }
        memories = unlinked_memories;
    }
    memories.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    memories.truncate(limit.clamp(1, 500));
    let memories = memories
        .into_iter()
        .map(|memory| GraphCandidateMemory {
            id: memory.id,
            name: memory.name,
            r#type: memory.r#type,
            scope: memory.scope,
            tags: parse_string_array(&memory.tags).unwrap_or_default(),
            content: memory.content,
        })
        .collect();
    Ok(GraphCandidates {
        status: "ok".to_string(),
        schema_version: GRAPH_SCHEMA_VERSION,
        instructions: vec![
            "Generate semantic edges only when useful.".to_string(),
            "Use EXTRACTED when explicitly stated.".to_string(),
            "Use INFERRED for reasonable relations.".to_string(),
            "Use AMBIGUOUS for uncertain relations requiring review.".to_string(),
            "Every edge needs evidence; do not include secrets.".to_string(),
        ],
        allowed_relations: SEMANTIC_RELATIONS
            .iter()
            .map(|relation| relation.to_string())
            .collect(),
        memories,
    })
}

fn memory_has_useful_graph_edge(conn: &Connection, memory_id: &str) -> Result<bool> {
    let node_id = memory_node_id(memory_id);
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM graph_edges
         WHERE status = 'active'
           AND (source_node_id = ?1 OR target_node_id = ?1)
           AND relation NOT IN ('has_type', 'in_scope', 'from_source')",
        params![node_id],
        |row| row.get(0),
    )?;
    Ok(count > 0)
}

pub fn resolve_query_start_nodes(
    conn: &Connection,
    query: &str,
    memory_hits: &[(String, f64)],
    scope_filter: Option<&[String]>,
    limit: usize,
) -> Result<Vec<GraphQueryStart>> {
    if query.chars().count() > 1_000 {
        bail!("graph query cannot exceed 1000 characters");
    }
    let mut scores = HashMap::<String, f64>::new();
    if let Ok(node_id) = resolve_node_id(conn, query, scope_filter) {
        scores.insert(node_id, 1.25);
    }
    for (memory_id, score) in memory_hits {
        let node_id = memory_node_id(memory_id);
        if node_by_id(conn, &node_id)?.is_some() {
            scores
                .entry(node_id)
                .and_modify(|current| *current = current.max(*score))
                .or_insert(*score);
        }
    }

    let pattern = format!("%{}%", escape_like_pattern(query.trim()));
    let mut stmt = conn.prepare(
        "SELECT id, scope FROM graph_nodes
         WHERE id LIKE ?1 ESCAPE '\\' OR label LIKE ?1 ESCAPE '\\'
         ORDER BY CASE kind
             WHEN 'memory' THEN 0
             WHEN 'concept' THEN 1
             WHEN 'tag' THEN 2
             ELSE 3
         END, id
         LIMIT ?2",
    )?;
    let rows = stmt.query_map(params![pattern, (limit * 3).max(limit) as i64], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
    })?;
    for row in rows {
        let (id, scope) = row?;
        if scope_allowed(scope.as_deref(), scope_filter) {
            scores.entry(id).or_insert(0.55);
        }
    }
    let mut starts = scores
        .into_iter()
        .map(|(node_id, score)| GraphQueryStart { node_id, score })
        .collect::<Vec<_>>();
    starts.sort_by(|left, right| {
        right
            .score
            .partial_cmp(&left.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| left.node_id.cmp(&right.node_id))
    });
    starts.truncate(limit.max(1));
    Ok(starts)
}

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
fn traversal_edges(conn: &Connection, options: GraphPathOptions) -> Result<Vec<GraphEdge>> {
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

fn traversal_adjacency(
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

fn resolve_node_id(
    conn: &Connection,
    reference: &str,
    scope_filter: Option<&[String]>,
) -> Result<String> {
    let reference = reference.trim();
    if node_by_id(conn, reference)?.is_some() {
        return Ok(reference.to_string());
    }
    if let Some(memory_ref) = reference.strip_prefix("memory:") {
        let node_id = memory_node_id(memory_ref);
        if node_by_id(conn, &node_id)?.is_some() {
            return Ok(node_id);
        }
    }
    if let Some(id) = reference.strip_prefix("id:") {
        if let Some(memory) = memory_by_id(conn, id)? {
            let node_id = memory_node_id(&memory.id);
            if node_by_id(conn, &node_id)?.is_some() {
                return Ok(node_id);
            }
        }
    }
    if scope_filter.is_none() {
        if let Some(memory) = memory_by_id(conn, reference)? {
            let node_id = memory_node_id(&memory.id);
            if node_by_id(conn, &node_id)?.is_some() {
                return Ok(node_id);
            }
        }
    }
    let named_memory = match scope_filter {
        Some(scopes) => {
            let mut project_matches = Vec::new();
            let mut global_match = None;
            for scope in scopes {
                if let Some(memory) = memory_by_name_in_scope(conn, reference, scope)? {
                    if scope == "global" {
                        global_match = Some(memory);
                    } else {
                        project_matches.push(memory);
                    }
                }
            }
            match project_matches.as_slice() {
                [memory] => Some(memory.clone()),
                [] => global_match,
                _ => bail!("graph memory reference is ambiguous across scopes: {reference}"),
            }
        }
        None => memory_by_name(conn, reference)?,
    };
    if let Some(memory) = named_memory {
        let node_id = memory_node_id(&memory.id);
        if node_by_id(conn, &node_id)?.is_some() {
            return Ok(node_id);
        }
    }
    if let Some(memory) = memory_by_id(conn, reference)? {
        let node_id = memory_node_id(&memory.id);
        if node_by_id(conn, &node_id)?.is_some() {
            return Ok(node_id);
        }
    }

    let mut stmt = conn.prepare("SELECT id FROM graph_nodes WHERE label = ?1 ORDER BY id")?;
    let rows = stmt.query_map(params![reference], |row| row.get::<_, String>(0))?;
    let matches = rows.collect::<rusqlite::Result<Vec<_>>>()?;
    match matches.as_slice() {
        [node_id] => Ok(node_id.clone()),
        [] => bail!("graph node not found: {reference}"),
        _ => bail!("graph node reference is ambiguous: {reference}"),
    }
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

fn row_to_edge(row: &rusqlite::Row<'_>) -> rusqlite::Result<GraphEdge> {
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
fn scope_allowed(scope: Option<&str>, scope_filter: Option<&[String]>) -> bool {
    let Some(filter) = scope_filter else {
        return true;
    };
    match scope {
        Some(scope) => filter.iter().any(|candidate| candidate == scope),
        None => true,
    }
}

fn confidence_score(confidence: &str) -> f64 {
    match confidence {
        "EXTRACTED" => 1.0,
        "INFERRED" => 0.7,
        "AMBIGUOUS" => 0.3,
        _ => 0.1,
    }
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

fn escape_like_pattern(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}
