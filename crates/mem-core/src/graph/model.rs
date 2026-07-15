//! Public graph request, report, node, and edge models.

use std::collections::{BTreeMap, HashMap};

use serde::Serialize;
use serde_json::Value;

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
