use anyhow::Result;
use rusqlite::Connection;
use serde_json::Value;

use crate::db::Memory;

use super::super::ids::{artifact_node_id, stable_hash_hex};
use super::super::model::GraphEdge;
use super::super::store::{insert_edge, insert_simple_node};
use super::super::{ACTIVE, DETERMINISTIC, EXTRACTED};

#[allow(clippy::too_many_arguments)]
pub(super) fn insert_edge_simple(
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

pub(super) fn insert_artifact_node(
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

pub(super) fn memory_is_active(memory: &Memory) -> bool {
    memory.valid_until.is_none()
}
