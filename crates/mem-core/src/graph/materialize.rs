//! Deterministic graph rebuild and source extraction.

use std::collections::HashMap;
use std::path::Path;

use anyhow::Result;
use rusqlite::Connection;
use serde_json::{json, Value};
use serde_yaml::Value as YamlValue;

use crate::artifact::ArtifactManifest;
use crate::db::{graph_memories, with_transaction, Memory};
use crate::util::{extract_claims, now, parse_string_array, ClaimKind};

use super::ids::{
    artifact_node_id, memory_node_id, safe_node_part, scope_node_id, source_node_id,
    stable_hash_hex, tag_node_id, type_node_id, workflow_step_node_id,
};
use super::model::*;
use super::semantic::materialize_semantic_edges;
use super::store::{
    count_edges_by_origin, count_table, graph_dirty, insert_edge, insert_node, insert_simple_node,
    metadata_value, node_by_id, set_metadata, table_exists,
};
use super::{
    ACTIVE, DETERMINISTIC, EXTRACTED, GRAPH_DIRTY_KEY, GRAPH_LAST_REBUILT_AT_KEY,
    GRAPH_SCHEMA_VERSION, GRAPH_SCHEMA_VERSION_KEY,
};

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
