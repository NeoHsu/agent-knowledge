use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::path::Path;

use anyhow::{bail, Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use serde_yaml::Value as YamlValue;

use crate::artifact::ArtifactManifest;
use crate::db::{
    add_ambiguity_record, graph_memories, memory_by_id, memory_by_name, memory_by_name_in_scope,
    new_event_uid, resolve_ambiguity_record, with_transaction, Memory,
};
use crate::util::{
    extract_claims, normalize_rfc3339, now, parse_string_array, sanitize_secret_field,
    source_priority, strip_secrets, ClaimKind,
};

mod semantic;

use semantic::materialize_semantic_edges;
pub use semantic::{
    ingest_semantic_edges, merge_semantic_edges, review_semantic_edges, set_semantic_edge_status,
};

pub const GRAPH_SCHEMA_VERSION: i64 = 1;
pub const GRAPH_DIRTY_KEY: &str = "graph_dirty";
pub const GRAPH_SCHEMA_VERSION_KEY: &str = "graph_schema_version";
pub const GRAPH_LAST_REBUILT_AT_KEY: &str = "graph_last_rebuilt_at";

const DETERMINISTIC: &str = "deterministic";
const SEMANTIC: &str = "semantic";
const EXTRACTED: &str = "EXTRACTED";
const ACTIVE: &str = "active";

pub const SEMANTIC_RELATIONS: &[&str] = &[
    "same_theme",
    "refines",
    "contradicts",
    "depends_on",
    "blocks",
    "risk_for",
    "policy_for",
    "procedure_for",
    "evidence_for",
    "applies_to",
    "mentions_concept",
    "related_to",
];

#[derive(Debug, Clone, Serialize)]
pub struct GraphNode {
    pub id: String,
    pub kind: String,
    pub label: String,
    pub ref_table: Option<String>,
    pub ref_id: Option<String>,
    pub scope: Option<String>,
    pub metadata: Value,
    pub origin: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct GraphEdge {
    pub id: String,
    pub source_node_id: String,
    pub target_node_id: String,
    pub relation: String,
    pub confidence: String,
    pub status: String,
    pub evidence: Option<String>,
    pub source_ref: Option<String>,
    pub scope: Option<String>,
    pub weight: f64,
    pub origin: String,
    pub metadata: Value,
}

#[derive(Debug, Clone, Serialize)]
pub struct GraphRebuildReport {
    pub status: String,
    pub schema_version: i64,
    pub nodes: usize,
    pub edges: usize,
    pub deterministic_edges: usize,
    pub semantic_edges: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct GraphStats {
    pub status: String,
    pub schema_version: i64,
    pub nodes: usize,
    pub edges: usize,
    pub by_kind: BTreeMap<String, usize>,
    pub by_confidence: BTreeMap<String, usize>,
    pub by_status: BTreeMap<String, usize>,
    pub pending_edges: usize,
    pub dirty: bool,
    pub last_rebuilt_at: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct GraphExplain {
    pub status: String,
    pub node: GraphNode,
    pub neighbors: Vec<GraphNeighbor>,
}

#[derive(Debug, Clone, Serialize)]
pub struct GraphNeighbor {
    pub direction: String,
    pub relation: String,
    pub confidence: String,
    pub status: String,
    pub edge_id: String,
    pub node: GraphNode,
    pub evidence: Option<String>,
}

#[derive(Debug, Clone, Copy)]
pub enum GraphConfidenceFilter {
    Extracted,
    Inferred,
    All,
}

#[derive(Debug, Clone, Copy)]
pub enum GraphDirection {
    Any,
    Outgoing,
    Incoming,
}

#[derive(Debug, Clone)]
pub struct GraphPathOptions {
    pub max_depth: usize,
    pub include_ambiguous: bool,
    pub include_metadata: bool,
    pub confidence: GraphConfidenceFilter,
    pub direction: GraphDirection,
    pub scope_filter: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct GraphPathReport {
    pub status: String,
    pub from: GraphNode,
    pub to: GraphNode,
    pub hops: usize,
    pub path_score: f64,
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<GraphPathEdge>,
}

#[derive(Debug, Clone, Serialize)]
pub struct GraphPathEdge {
    pub id: String,
    pub source: String,
    pub target: String,
    pub relation: String,
    pub confidence: String,
    pub status: String,
    pub evidence: Option<String>,
    pub traversed_from: String,
    pub traversed_to: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct GraphExport {
    pub schema_version: i64,
    pub nodes: Vec<GraphExportNode>,
    pub edges: Vec<GraphExportEdge>,
}

#[derive(Debug, Clone, Serialize)]
pub struct GraphExportNode {
    pub id: String,
    pub label: String,
    pub kind: String,
    pub metadata: Value,
}

#[derive(Debug, Clone, Serialize)]
pub struct GraphExportEdge {
    pub source: String,
    pub target: String,
    pub relation: String,
    pub confidence: String,
    pub status: String,
    pub evidence: Option<String>,
    pub metadata: Value,
}

#[derive(Debug, Clone, Serialize)]
pub struct GraphCandidates {
    pub status: String,
    pub schema_version: i64,
    pub instructions: Vec<String>,
    pub allowed_relations: Vec<String>,
    pub memories: Vec<GraphCandidateMemory>,
}

#[derive(Debug, Clone, Serialize)]
pub struct GraphCandidateMemory {
    pub id: String,
    pub name: String,
    pub r#type: String,
    pub scope: String,
    pub tags: Vec<String>,
    pub content: Option<String>,
}

#[derive(Debug, Clone)]
pub struct GraphIngestOptions {
    pub pending_inferred: bool,
    pub source: String,
    pub user_confirmed: bool,
    pub allow_secret_redaction: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct GraphIngestReport {
    pub status: String,
    pub total: usize,
    pub inserted: usize,
    pub updated: usize,
    pub unchanged: usize,
    pub rejected: usize,
    pub pending: usize,
    pub results: Vec<GraphIngestResult>,
}

#[derive(Debug, Clone, Serialize)]
pub struct GraphIngestResult {
    pub index: usize,
    pub status: String,
    pub id: Option<String>,
    pub reason: Option<String>,
    pub source: Option<String>,
    pub target: Option<String>,
    pub relation: Option<String>,
    pub confidence: Option<String>,
    pub edge_status: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct GraphSemanticEdgeRow {
    pub id: String,
    pub source_ref: String,
    pub source_label: Option<String>,
    pub target_ref: String,
    pub target_label: Option<String>,
    pub relation: String,
    pub confidence: String,
    pub status: String,
    pub evidence: String,
    pub rationale: Option<String>,
    pub source_spans: Value,
    pub tags: Value,
    pub generated_by: String,
    pub source: String,
    pub user_confirmed_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub valid_until: Option<String>,
    pub ambiguity_id: Option<i64>,
    pub version: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct GraphReviewReport {
    pub status: String,
    pub edges: Vec<GraphSemanticEdgeRow>,
}

#[derive(Debug, Clone, Serialize)]
pub struct GraphSemanticUpdateReport {
    pub status: String,
    pub id: String,
    pub edge_status: String,
    pub ambiguity_id: Option<i64>,
}

#[derive(Debug, Default, Clone, Serialize)]
pub struct GraphSemanticMergeReport {
    pub imported: usize,
    pub identical: usize,
    pub conflicts: usize,
    pub trusted_updates: usize,
    pub rejected_lower_trust: usize,
    pub unattested_manual_downgraded: usize,
    pub unresolved_endpoints: usize,
    pub pending: usize,
    #[serde(skip)]
    pub edge_id_map: HashMap<String, String>,
}

impl GraphSemanticMergeReport {
    pub fn changed(&self) -> bool {
        self.imported > 0 || self.trusted_updates > 0 || self.conflicts > 0
    }
}

#[derive(Debug, Clone)]
pub struct GraphQueryStart {
    pub node_id: String,
    pub score: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct GraphQueryReport {
    pub status: String,
    pub query: String,
    pub start_nodes: Vec<GraphNode>,
    pub nodes: Vec<GraphQueryNode>,
    pub edges: Vec<GraphQueryEdge>,
}

#[derive(Debug, Clone, Serialize)]
pub struct GraphQueryNode {
    pub node: GraphNode,
    pub depth: usize,
    pub score: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct GraphQueryEdge {
    pub id: String,
    pub source: String,
    pub target: String,
    pub relation: String,
    pub confidence: String,
    pub status: String,
    pub evidence: Option<String>,
    pub score: f64,
}

#[derive(Debug, Clone)]
pub struct GraphQueryOptions {
    pub depth: usize,
    pub limit: usize,
    pub include_ambiguous: bool,
    pub include_metadata: bool,
    pub confidence: GraphConfidenceFilter,
    pub direction: GraphDirection,
    pub scope_filter: Option<Vec<String>>,
}

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

pub fn rebuild(conn: &Connection, root: &Path) -> Result<GraphRebuildReport> {
    let manifest = ArtifactManifest::load(root)?;
    with_transaction(conn, |conn| {
        conn.execute("DELETE FROM graph_edges", [])?;
        conn.execute("DELETE FROM graph_nodes", [])?;

        let memories = graph_memories(conn)?;
        let memory_index = memories
            .iter()
            .map(|memory| (memory.id.clone(), memory.clone()))
            .collect::<HashMap<_, _>>();

        for memory in &memories {
            add_memory_metadata(conn, memory, &memory_index)?;
        }
        for memory in &memories {
            if !memory_is_active(memory) {
                continue;
            }
            add_claim_edges(conn, memory)?;
            if memory.r#type == "workflow" {
                add_workflow_edges(conn, memory)?;
            }
        }
        if let Some(manifest) = manifest.as_ref() {
            add_artifact_manifest(conn, manifest)?;
        }
        add_workflow_run_edges(conn, &memory_index)?;
        let semantic_edges = materialize_semantic_edges(conn)?;

        set_metadata(
            conn,
            GRAPH_SCHEMA_VERSION_KEY,
            &GRAPH_SCHEMA_VERSION.to_string(),
        )?;
        set_metadata(conn, GRAPH_DIRTY_KEY, "false")?;
        set_metadata(conn, GRAPH_LAST_REBUILT_AT_KEY, &now())?;

        let nodes = count_table(conn, "graph_nodes")?;
        let edges = count_table(conn, "graph_edges")?;
        let deterministic_edges = count_edges_by_origin(conn, DETERMINISTIC)?;
        Ok(GraphRebuildReport {
            status: "rebuilt".to_string(),
            schema_version: GRAPH_SCHEMA_VERSION,
            nodes,
            edges,
            deterministic_edges,
            semantic_edges,
        })
    })
}

/// Rebuild the materialized graph only when its schema or source state is stale.
pub fn ensure_fresh(conn: &Connection, root: &Path) -> Result<Option<GraphRebuildReport>> {
    let schema_matches = metadata_value(conn, GRAPH_SCHEMA_VERSION_KEY)?
        .and_then(|value| value.parse::<i64>().ok())
        == Some(GRAPH_SCHEMA_VERSION);
    let materialization_present =
        table_exists(conn, "graph_nodes")? && table_exists(conn, "graph_edges")?;
    if !materialization_present {
        ensure_materialized_schema(conn)?;
    }
    if graph_dirty(conn)? || !schema_matches || !materialization_present {
        return rebuild(conn, root).map(Some);
    }
    Ok(None)
}

fn ensure_materialized_schema(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS graph_nodes (
             id TEXT PRIMARY KEY,
             kind TEXT NOT NULL,
             label TEXT NOT NULL,
             ref_table TEXT,
             ref_id TEXT,
             scope TEXT,
             metadata TEXT NOT NULL DEFAULT '{}' CHECK (json_valid(metadata) AND json_type(metadata) = 'object'),
             origin TEXT NOT NULL CHECK (origin IN ('deterministic', 'semantic')),
             created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
             updated_at DATETIME DEFAULT CURRENT_TIMESTAMP
         );
         CREATE TABLE IF NOT EXISTS graph_edges (
             id TEXT PRIMARY KEY,
             source_node_id TEXT NOT NULL,
             target_node_id TEXT NOT NULL,
             relation TEXT NOT NULL,
             confidence TEXT NOT NULL CHECK (confidence IN ('EXTRACTED', 'INFERRED', 'AMBIGUOUS')),
             status TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'pending', 'rejected', 'superseded')),
             evidence TEXT,
             source_ref TEXT,
             scope TEXT,
             weight REAL NOT NULL DEFAULT 1.0,
             origin TEXT NOT NULL CHECK (origin IN ('deterministic', 'semantic')),
             metadata TEXT NOT NULL DEFAULT '{}' CHECK (json_valid(metadata) AND json_type(metadata) = 'object'),
             created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
             updated_at DATETIME DEFAULT CURRENT_TIMESTAMP,
             FOREIGN KEY(source_node_id) REFERENCES graph_nodes(id),
             FOREIGN KEY(target_node_id) REFERENCES graph_nodes(id)
         );
         CREATE INDEX IF NOT EXISTS idx_graph_nodes_kind ON graph_nodes(kind);
         CREATE INDEX IF NOT EXISTS idx_graph_nodes_label ON graph_nodes(label);
         CREATE INDEX IF NOT EXISTS idx_graph_edges_source ON graph_edges(source_node_id);
         CREATE INDEX IF NOT EXISTS idx_graph_edges_target ON graph_edges(target_node_id);
         CREATE INDEX IF NOT EXISTS idx_graph_edges_relation ON graph_edges(relation);
         CREATE INDEX IF NOT EXISTS idx_graph_edges_status ON graph_edges(status);
         CREATE INDEX IF NOT EXISTS idx_graph_edges_confidence ON graph_edges(confidence);",
    )?;
    Ok(())
}

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

pub fn graph_dirty(conn: &Connection) -> Result<bool> {
    Ok(metadata_value(conn, GRAPH_DIRTY_KEY)?
        .map(|value| matches!(value.as_str(), "true" | "1"))
        .unwrap_or(false))
}

pub fn set_graph_dirty(conn: &Connection, dirty: bool) -> Result<()> {
    set_metadata(conn, GRAPH_DIRTY_KEY, if dirty { "true" } else { "false" })
}

fn add_memory_metadata(
    conn: &Connection,
    memory: &Memory,
    memory_index: &HashMap<String, Memory>,
) -> Result<()> {
    let memory_node = memory_node_id(&memory.id);
    insert_node(
        conn,
        &GraphNode {
            id: memory_node.clone(),
            kind: "memory".to_string(),
            label: memory.name.clone(),
            ref_table: Some("memories".to_string()),
            ref_id: Some(memory.id.clone()),
            scope: Some(memory.scope.clone()),
            metadata: json!({
                "name": memory.name,
                "type": memory.r#type,
                "source": memory.source,
                "confidence": memory.confidence,
                "tags": parse_string_array(&memory.tags).unwrap_or_default(),
                "lifecycle": if memory_is_active(memory) { "active" } else { "superseded" },
                "valid_until": memory.valid_until,
                "superseded_by": memory.superseded_by,
            }),
            origin: DETERMINISTIC.to_string(),
        },
    )?;

    let type_id = type_node_id(&memory.r#type);
    insert_simple_node(
        conn,
        &type_id,
        "type",
        &memory.r#type,
        None,
        DETERMINISTIC,
        json!({}),
    )?;
    insert_edge_simple(
        conn,
        &memory_node,
        &type_id,
        "has_type",
        "memory type metadata",
        Some(&memory.id),
        Some(&memory.scope),
        0.2,
        DETERMINISTIC,
        json!({}),
    )?;

    let scope_id = scope_node_id(&memory.scope);
    insert_simple_node(
        conn,
        &scope_id,
        "scope",
        &memory.scope,
        Some(&memory.scope),
        DETERMINISTIC,
        json!({}),
    )?;
    insert_edge_simple(
        conn,
        &memory_node,
        &scope_id,
        "in_scope",
        "memory scope metadata",
        Some(&memory.id),
        Some(&memory.scope),
        0.2,
        DETERMINISTIC,
        json!({}),
    )?;

    let source_id = source_node_id(&memory.source);
    insert_simple_node(
        conn,
        &source_id,
        "source",
        &memory.source,
        None,
        DETERMINISTIC,
        json!({}),
    )?;
    insert_edge_simple(
        conn,
        &memory_node,
        &source_id,
        "from_source",
        "memory source metadata",
        Some(&memory.id),
        Some(&memory.scope),
        0.2,
        DETERMINISTIC,
        json!({}),
    )?;

    if memory_is_active(memory) {
        for tag in parse_string_array(&memory.tags).unwrap_or_default() {
            let tag_id = tag_node_id(&tag);
            insert_simple_node(conn, &tag_id, "tag", &tag, None, DETERMINISTIC, json!({}))?;
            insert_edge_simple(
                conn,
                &memory_node,
                &tag_id,
                "has_tag",
                "memory tag metadata",
                Some(&memory.id),
                Some(&memory.scope),
                0.7,
                DETERMINISTIC,
                json!({}),
            )?;
        }
    }

    if let Some(target) = memory
        .superseded_by
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        let target_id = memory_node_id(target);
        if node_by_id(conn, &target_id)?.is_none() {
            if let Some(target_memory) = memory_index.get(target) {
                insert_simple_node(
                    conn,
                    &target_id,
                    "memory",
                    &target_memory.name,
                    Some(&target_memory.scope),
                    DETERMINISTIC,
                    json!({
                        "name": target_memory.name,
                        "type": target_memory.r#type,
                        "lifecycle": if memory_is_active(target_memory) { "active" } else { "superseded" }
                    }),
                )?;
            } else {
                insert_simple_node(
                    conn,
                    &target_id,
                    "memory",
                    target,
                    None,
                    DETERMINISTIC,
                    json!({"dangling": true}),
                )?;
            }
        }
        insert_edge_simple(
            conn,
            &memory_node,
            &target_id,
            "superseded_by",
            "memory superseded_by metadata",
            Some(&memory.id),
            Some(&memory.scope),
            1.0,
            DETERMINISTIC,
            json!({}),
        )?;
    }

    Ok(())
}

fn add_claim_edges(conn: &Connection, memory: &Memory) -> Result<()> {
    let Some(content) = memory.content.as_deref() else {
        return Ok(());
    };
    let memory_node_id = memory_node_id(&memory.id);
    for claim in extract_claims(content).claims {
        match claim.kind {
            ClaimKind::Path => {
                let claim_id = format!("claim:path:{}", stable_hash_hex(&claim.text));
                insert_simple_node(
                    conn,
                    &claim_id,
                    "claim_path",
                    &claim.text,
                    Some(&memory.scope),
                    DETERMINISTIC,
                    json!({"claim": claim.text, "backticked": claim.backticked}),
                )?;
                insert_edge_simple(
                    conn,
                    &memory_node_id,
                    &claim_id,
                    "mentions_path",
                    "path claim extracted from memory content",
                    Some(&memory.id),
                    Some(&memory.scope),
                    0.8,
                    DETERMINISTIC,
                    json!({"backticked": claim.backticked}),
                )?;
            }
            ClaimKind::Command => {
                let claim_id = format!("claim:command:{}", safe_node_part(&claim.text));
                insert_simple_node(
                    conn,
                    &claim_id,
                    "claim_command",
                    &claim.text,
                    Some(&memory.scope),
                    DETERMINISTIC,
                    json!({"claim": claim.text, "backticked": claim.backticked}),
                )?;
                insert_edge_simple(
                    conn,
                    &memory_node_id,
                    &claim_id,
                    "mentions_command",
                    "command claim extracted from memory content",
                    Some(&memory.id),
                    Some(&memory.scope),
                    0.8,
                    DETERMINISTIC,
                    json!({"backticked": claim.backticked}),
                )?;
            }
        }
    }
    Ok(())
}

fn add_workflow_edges(conn: &Connection, memory: &Memory) -> Result<()> {
    let Some(content) = memory.content.as_deref() else {
        return Ok(());
    };
    let Ok(value) = serde_yaml::from_str::<YamlValue>(content) else {
        return Ok(());
    };
    let Some(mapping) = value.as_mapping() else {
        return Ok(());
    };
    let memory_node_id = memory_node_id(&memory.id);

    if let Some(reusable_scripts) =
        yaml_get(mapping, "reusable_scripts").and_then(YamlValue::as_sequence)
    {
        for (index, script) in reusable_scripts.iter().enumerate() {
            let Some(script) = script.as_mapping() else {
                continue;
            };
            let Some(path) = yaml_get(script, "path").and_then(YamlValue::as_str) else {
                continue;
            };
            let owner = yaml_get(script, "owner")
                .and_then(YamlValue::as_str)
                .unwrap_or_default();
            let required = yaml_get(script, "required")
                .and_then(YamlValue::as_bool)
                .unwrap_or(false);
            let artifact_id = artifact_node_id(path);
            insert_artifact_node(
                conn,
                path,
                Some(&memory.scope),
                json!({"owner": owner, "required": required, "workflow": memory.id}),
            )?;
            insert_edge_simple(
                conn,
                &memory_node_id,
                &artifact_id,
                "references_artifact",
                "workflow reusable_scripts entry",
                Some(&memory.id),
                Some(&memory.scope),
                1.1,
                DETERMINISTIC,
                json!({"owner": owner, "required": required, "index": index}),
            )?;
        }
    }

    if let Some(steps) = yaml_get(mapping, "steps").and_then(YamlValue::as_sequence) {
        for (index, step) in steps.iter().enumerate() {
            let Some(step) = step.as_mapping() else {
                continue;
            };
            let step_id = yaml_get(step, "id")
                .and_then(YamlValue::as_str)
                .filter(|id| !id.trim().is_empty())
                .map(str::to_string)
                .unwrap_or_else(|| format!("step_{}", index + 1));
            let step_node_id = workflow_step_node_id(&memory.id, &step_id);
            let run = yaml_get(step, "run")
                .and_then(YamlValue::as_str)
                .map(str::to_string);
            let confirm = yaml_get(step, "confirm")
                .and_then(YamlValue::as_bool)
                .unwrap_or(false);
            insert_node(
                conn,
                &GraphNode {
                    id: step_node_id.clone(),
                    kind: "workflow_step".to_string(),
                    label: step_id.clone(),
                    ref_table: Some("memories".to_string()),
                    ref_id: Some(memory.id.clone()),
                    scope: Some(memory.scope.clone()),
                    metadata: json!({
                        "workflow": memory.id,
                        "step_id": step_id,
                        "index": index,
                        "run": run,
                        "confirm": confirm,
                    }),
                    origin: DETERMINISTIC.to_string(),
                },
            )?;
            insert_edge_simple(
                conn,
                &memory_node_id,
                &step_node_id,
                "has_workflow_step",
                "workflow steps entry",
                Some(&memory.id),
                Some(&memory.scope),
                1.0,
                DETERMINISTIC,
                json!({"index": index}),
            )?;
            let run_artifact = yaml_get(step, "run")
                .and_then(YamlValue::as_str)
                .and_then(|run| first_artifact_token(run).map(|path| (run, path)));
            if let Some((run, path)) = run_artifact {
                let artifact_id = artifact_node_id(&path);
                insert_artifact_node(
                    conn,
                    &path,
                    Some(&memory.scope),
                    json!({"workflow": memory.id}),
                )?;
                insert_edge_simple(
                    conn,
                    &step_node_id,
                    &artifact_id,
                    "step_uses_artifact",
                    "workflow step run starts with artifact path",
                    Some(&memory.id),
                    Some(&memory.scope),
                    1.1,
                    DETERMINISTIC,
                    json!({"run": run}),
                )?;
            }
            if confirm {
                let concept_id = "concept:confirmation_required".to_string();
                insert_simple_node(
                    conn,
                    &concept_id,
                    "concept",
                    "confirmation_required",
                    None,
                    DETERMINISTIC,
                    json!({}),
                )?;
                insert_edge_simple(
                    conn,
                    &step_node_id,
                    &concept_id,
                    "requires_confirmation",
                    "workflow step confirm flag is true",
                    Some(&memory.id),
                    Some(&memory.scope),
                    1.0,
                    DETERMINISTIC,
                    json!({}),
                )?;
            }
        }
    }

    Ok(())
}

fn add_artifact_manifest(conn: &Connection, manifest: &ArtifactManifest) -> Result<()> {
    for entry in manifest.entries() {
        let path = entry.record.path.clone();
        let artifact_id = artifact_node_id(&path);
        insert_node(
            conn,
            &GraphNode {
                id: artifact_id.clone(),
                kind: "artifact".to_string(),
                label: path.clone(),
                ref_table: Some("manifest".to_string()),
                ref_id: Some(entry.name.clone()),
                scope: Some(entry.record.scope.clone()),
                metadata: json!({
                    "manifest_entry": entry.name,
                    "kind": entry.record.kind,
                    "checksum": entry.record.checksum,
                    "executable": entry.record.executable.unwrap_or(false),
                    "description": entry.record.description,
                    "tags": entry.record.tags.clone().unwrap_or_default(),
                }),
                origin: DETERMINISTIC.to_string(),
            },
        )?;
        let scope_id = scope_node_id(&entry.record.scope);
        insert_simple_node(
            conn,
            &scope_id,
            "scope",
            &entry.record.scope,
            Some(&entry.record.scope),
            DETERMINISTIC,
            json!({}),
        )?;
        insert_edge_simple(
            conn,
            &artifact_id,
            &scope_id,
            "in_scope",
            "artifact manifest scope metadata",
            Some(&path),
            Some(&entry.record.scope),
            0.2,
            DETERMINISTIC,
            json!({}),
        )?;
        for tag in entry.record.tags.unwrap_or_default() {
            let tag_id = tag_node_id(&tag);
            insert_simple_node(conn, &tag_id, "tag", &tag, None, DETERMINISTIC, json!({}))?;
            insert_edge_simple(
                conn,
                &artifact_id,
                &tag_id,
                "has_tag",
                "artifact manifest tag metadata",
                Some(&path),
                Some(&entry.record.scope),
                0.7,
                DETERMINISTIC,
                json!({}),
            )?;
        }
    }
    Ok(())
}

fn add_workflow_run_edges(conn: &Connection, memory_index: &HashMap<String, Memory>) -> Result<()> {
    let mut stmt = conn.prepare(
        "SELECT id, memory_id, result, note, source, created_at
         FROM workflow_runs
         ORDER BY id",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, Option<String>>(3)?,
            row.get::<_, Option<String>>(4)?,
            row.get::<_, String>(5)?,
        ))
    })?;
    for row in rows {
        let (id, memory_id, result, note, source, created_at) = row?;
        let Some(workflow) = memory_index
            .get(&memory_id)
            .filter(|memory| memory_is_active(memory))
        else {
            continue;
        };
        let scope = Some(workflow.scope.clone());
        let workflow_label = workflow.name.clone();
        let run_id = format!("workflow_run:{id}");
        insert_node(
            conn,
            &GraphNode {
                id: run_id.clone(),
                kind: "workflow_run".to_string(),
                label: format!("{workflow_label} run {id}: {result}"),
                ref_table: Some("workflow_runs".to_string()),
                ref_id: Some(id.to_string()),
                scope: scope.clone(),
                metadata: json!({
                    "memory_id": memory_id,
                    "result": result,
                    "note": note,
                    "source": source,
                    "created_at": created_at,
                }),
                origin: DETERMINISTIC.to_string(),
            },
        )?;
        let memory_node = memory_node_id(&memory_id);
        insert_edge_simple(
            conn,
            &memory_node,
            &run_id,
            "recorded_run",
            "workflow run history record",
            Some(&memory_id),
            scope.as_deref(),
            1.0,
            DETERMINISTIC,
            json!({}),
        )?;
        let result_concept = format!("concept:run_{}", safe_node_part(&result));
        insert_simple_node(
            conn,
            &result_concept,
            "concept",
            &format!("run_{result}"),
            None,
            DETERMINISTIC,
            json!({}),
        )?;
        insert_edge_simple(
            conn,
            &run_id,
            &result_concept,
            "has_result",
            "workflow run result",
            Some(&memory_id),
            scope.as_deref(),
            1.0,
            DETERMINISTIC,
            json!({}),
        )?;
    }
    Ok(())
}

fn table_exists(conn: &Connection, table: &str) -> Result<bool> {
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
        params![table],
        |row| row.get(0),
    )?;
    Ok(count > 0)
}

#[allow(clippy::too_many_arguments)]
fn insert_edge_simple(
    conn: &Connection,
    source: &str,
    target: &str,
    relation: &str,
    evidence: &str,
    source_ref: Option<&str>,
    scope: Option<&str>,
    weight: f64,
    origin: &str,
    metadata: Value,
) -> Result<()> {
    let edge_id = format!(
        "edge:{}:{}",
        relation,
        stable_hash_hex(&format!(
            "{origin}\0{source}\0{target}\0{relation}\0{}",
            source_ref.unwrap_or_default()
        ))
    );
    insert_edge(
        conn,
        &GraphEdge {
            id: edge_id,
            source_node_id: source.to_string(),
            target_node_id: target.to_string(),
            relation: relation.to_string(),
            confidence: EXTRACTED.to_string(),
            status: ACTIVE.to_string(),
            evidence: Some(evidence.to_string()),
            source_ref: source_ref.map(str::to_string),
            scope: scope.map(str::to_string),
            weight,
            origin: origin.to_string(),
            metadata,
        },
    )
}

fn insert_simple_node(
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

fn insert_artifact_node(
    conn: &Connection,
    path: &str,
    scope: Option<&str>,
    metadata: Value,
) -> Result<()> {
    insert_simple_node(
        conn,
        &artifact_node_id(path),
        "artifact",
        path,
        scope,
        DETERMINISTIC,
        metadata,
    )
}

fn insert_node(conn: &Connection, node: &GraphNode) -> Result<()> {
    conn.execute(
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
        params![
            node.id,
            node.kind,
            node.label,
            node.ref_table,
            node.ref_id,
            node.scope,
            node.metadata.to_string(),
            node.origin,
        ],
    )?;
    Ok(())
}

fn insert_edge(conn: &Connection, edge: &GraphEdge) -> Result<()> {
    conn.execute(
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
        params![
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
        ],
    )?;
    Ok(())
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

fn relation_weight(relation: &str) -> f64 {
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

fn node_by_id(conn: &Connection, id: &str) -> Result<Option<GraphNode>> {
    conn.query_row(
        "SELECT id, kind, label, ref_table, ref_id, scope, metadata, origin
         FROM graph_nodes WHERE id = ?1",
        params![id],
        row_to_node,
    )
    .optional()
    .map_err(Into::into)
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

fn row_to_node(row: &rusqlite::Row<'_>) -> rusqlite::Result<GraphNode> {
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

fn count_table(conn: &Connection, table: &str) -> Result<usize> {
    let sql = format!("SELECT COUNT(*) FROM {table}");
    let count: i64 = conn.query_row(&sql, [], |row| row.get(0))?;
    Ok(count.max(0) as usize)
}

fn count_edges_by_origin(conn: &Connection, origin: &str) -> Result<usize> {
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

fn query_json_rows_local(conn: &Connection, sql: &str) -> Result<Vec<Value>> {
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

fn metadata_value(conn: &Connection, key: &str) -> Result<Option<String>> {
    conn.query_row(
        "SELECT value FROM metadata WHERE key = ?1",
        params![key],
        |row| row.get(0),
    )
    .optional()
    .map_err(Into::into)
}

fn set_metadata(conn: &Connection, key: &str, value: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO metadata (key, value, updated_at)
         VALUES (?1, ?2, ?3)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at
         WHERE metadata.value <> excluded.value",
        params![key, value, now()],
    )?;
    Ok(())
}

fn yaml_get<'a>(mapping: &'a serde_yaml::Mapping, key: &str) -> Option<&'a YamlValue> {
    mapping.get(YamlValue::String(key.to_string()))
}

fn first_artifact_token(run: &str) -> Option<String> {
    let token = run.split_whitespace().next()?.trim_matches(['"', '\'']);
    token.starts_with("artifacts/").then(|| token.to_string())
}

fn memory_is_active(memory: &Memory) -> bool {
    memory.valid_until.is_none()
}

fn memory_node_id(id: &str) -> String {
    format!("memory:{id}")
}

fn tag_node_id(tag: &str) -> String {
    format!("tag:{tag}")
}

fn scope_node_id(scope: &str) -> String {
    format!("scope:{scope}")
}

fn type_node_id(memory_type: &str) -> String {
    format!("type:{memory_type}")
}

fn source_node_id(source: &str) -> String {
    format!("source:{source}")
}

fn artifact_node_id(path: &str) -> String {
    format!("artifact:{path}")
}

fn workflow_step_node_id(memory_id: &str, step_id: &str) -> String {
    format!("workflow_step:{memory_id}:{}", safe_node_part(step_id))
}

fn safe_node_part(input: &str) -> String {
    let mut slug = String::new();
    for ch in input.chars() {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch.to_ascii_lowercase());
        } else if matches!(ch, '_' | '-' | '/' | ':' | '.') {
            slug.push(ch);
        } else if !slug.ends_with('_') {
            slug.push('_');
        }
    }
    let slug = slug.trim_matches('_').to_string();
    if slug.is_empty() {
        stable_hash_hex(input)
    } else {
        slug
    }
}

fn stable_hash_hex(input: &str) -> String {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in input.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

fn parse_json_value(raw: &str) -> Value {
    serde_json::from_str(raw).unwrap_or_else(|_| json!(raw))
}
