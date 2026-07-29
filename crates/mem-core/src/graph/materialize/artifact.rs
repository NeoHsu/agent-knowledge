use anyhow::Result;
use rusqlite::Connection;
use serde_json::json;

use crate::artifact::ArtifactManifest;

use super::super::ids::{artifact_node_id, scope_node_id, tag_node_id};
use super::super::model::GraphNode;
use super::super::store::{insert_node, insert_simple_node};
use super::super::DETERMINISTIC;
use super::support::insert_edge_simple;

pub(super) fn add_artifact_manifest(conn: &Connection, manifest: &ArtifactManifest) -> Result<()> {
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
