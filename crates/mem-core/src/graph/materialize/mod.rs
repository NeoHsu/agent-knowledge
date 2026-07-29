//! Deterministic graph rebuild and source extraction.

mod artifact;
mod memory;
mod support;
mod workflow;

use std::collections::HashMap;
use std::path::Path;

use anyhow::Result;
use rusqlite::Connection;

use crate::artifact::ArtifactManifest;
use crate::db::{graph_memories, with_transaction};
use crate::util::now;

use super::model::*;
use super::semantic::materialize_semantic_edges;
use super::store::{
    count_edges_by_origin, count_table, graph_dirty, metadata_value, set_metadata, table_exists,
};
use super::{
    DETERMINISTIC, GRAPH_DIRTY_KEY, GRAPH_LAST_REBUILT_AT_KEY, GRAPH_SCHEMA_VERSION,
    GRAPH_SCHEMA_VERSION_KEY,
};
use artifact::add_artifact_manifest;
use memory::{add_claim_edges, add_memory_metadata};
use support::memory_is_active;
use workflow::{add_workflow_edges, add_workflow_run_edges};

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
