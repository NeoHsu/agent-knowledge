use std::collections::HashMap;

use anyhow::Result;
use rusqlite::Connection;
use serde_json::json;

use crate::db::Memory;
use crate::util::{extract_claims, parse_string_array, ClaimKind};

use super::super::ids::{
    memory_node_id, safe_node_part, scope_node_id, source_node_id, stable_hash_hex, tag_node_id,
    type_node_id,
};
use super::super::model::GraphNode;
use super::super::store::{insert_node, insert_simple_node, node_by_id};
use super::super::DETERMINISTIC;
use super::support::{insert_edge_simple, memory_is_active};

pub(super) fn add_memory_metadata(
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

pub(super) fn add_claim_edges(conn: &Connection, memory: &Memory) -> Result<()> {
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
