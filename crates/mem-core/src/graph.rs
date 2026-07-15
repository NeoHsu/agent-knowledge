//! Public graph API and module wiring.
//!
//! Durable semantic assertions live in SQLite; deterministic nodes and edges
//! are rebuildable projections assembled by the modules below.

mod health;
mod ids;
mod materialize;
mod model;
mod query;
mod semantic;
mod store;

pub use health::graph_health;
pub use materialize::{ensure_fresh, rebuild};
pub use model::*;
pub use query::{
    candidates, explain, export_json, query_neighborhood, resolve_query_start_nodes, shortest_path,
};
pub use semantic::{
    ingest_semantic_edges, merge_semantic_edges, review_semantic_edges, set_semantic_edge_status,
};
pub use store::{graph_dirty, set_graph_dirty, stats};

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
