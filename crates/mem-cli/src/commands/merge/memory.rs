use std::collections::{HashMap, HashSet};

use super::sanitize::sanitize_incoming_memory;
use super::*;

#[derive(Debug, Default)]
pub(super) struct MemoryMergeState {
    pub(super) imported: usize,
    pub(super) identical: usize,
    pub(super) conflicts: usize,
    pub(super) trusted_updates: usize,
    pub(super) rejected_lower_trust: usize,
    pub(super) regenerated_ids: usize,
    pub(super) workflow_review_required: usize,
    pub(super) unattested_manual_downgraded: usize,
    pub(super) changed_index_ids: Vec<String>,
    pub(super) memory_id_map: HashMap<String, String>,
    pub(super) review_memory_ids: HashSet<String>,
}

pub(super) fn merge_memories(
    conn: &Connection,
    source_db: &Path,
    incoming_store: &str,
    incoming: Vec<Memory>,
    prefer_trusted: bool,
    allow_secret_redaction: bool,
) -> Result<MemoryMergeState> {
    let mut state = MemoryMergeState::default();

    for mut memory in incoming {
        sanitize_incoming_memory(&mut memory, incoming_store, allow_secret_redaction)?;
        if memory.source == "manual" && memory.user_confirmed_at.is_none() {
            memory.source = "agent".to_string();
            memory.confidence = "medium".to_string();
            memory.protected = false;
            state.unattested_manual_downgraded += 1;
        }
        if let Err(err) = workflow_core::validate_record_content(&memory) {
            add_workflow_review_record(conn, source_db, &memory, &err)?;
            state.workflow_review_required += 1;
            continue;
        }

        if let Some(existing) = memory_by_name_in_scope(conn, &memory.name, &memory.scope)? {
            state
                .memory_id_map
                .insert(memory.id.clone(), existing.id.clone());
            if normalized_text(existing.content.as_deref().unwrap_or_default())
                == normalized_text(memory.content.as_deref().unwrap_or_default())
            {
                merge_memory_usage(conn, &existing, &memory)?;
                state.identical += 1;
                continue;
            }

            let incoming_priority = source_priority(&memory.source);
            let existing_priority = source_priority(&existing.source);
            if incoming_priority < existing_priority {
                state.review_memory_ids.insert(memory.id.clone());
                state.rejected_lower_trust += 1;
                continue;
            }
            if prefer_trusted && incoming_priority > existing_priority {
                update_memory_from_merge(conn, &existing, &memory)?;
                state.changed_index_ids.push(existing.id.clone());
                state.trusted_updates += 1;
                continue;
            }

            state.review_memory_ids.insert(memory.id.clone());
            let context = serde_json::to_string(&json!({
                "kind": "merge_conflict",
                "source_store": incoming_store,
                "local": {
                    "id": &existing.id,
                    "name": &existing.name,
                    "scope": &existing.scope,
                    "source": &existing.source,
                    "priority": existing_priority,
                    "content": &existing.content
                },
                "incoming": {
                    "id": &memory.id,
                    "name": &memory.name,
                    "type": &memory.r#type,
                    "description": &memory.description,
                    "content": &memory.content,
                    "tags": &memory.tags,
                    "scope": &memory.scope,
                    "source": &memory.source,
                    "confidence": &memory.confidence,
                    "priority": incoming_priority,
                    "version": memory.version
                }
            }))?;
            add_ambiguity_record(
                conn,
                &format!("merge:{}:{}", memory.scope, memory.name),
                &[existing.id.clone(), memory.id.clone()],
                Some(&context),
            )?;
            state.conflicts += 1;
            continue;
        }

        let original_id = memory.id.clone();
        memory.id = unique_memory_id(conn, &memory.id)?;
        if memory.id != original_id {
            state.regenerated_ids += 1;
        }
        state.memory_id_map.insert(original_id, memory.id.clone());
        insert_memory_record(conn, &memory)?;
        log_change(
            conn,
            &memory.id,
            "merge",
            None,
            memory.content.as_deref(),
            "merge",
        )?;
        state.changed_index_ids.push(memory.id.clone());
        state.imported += 1;
    }

    Ok(state)
}

fn merge_memory_usage(conn: &Connection, existing: &Memory, incoming: &Memory) -> Result<()> {
    conn.execute(
        "UPDATE memories
         SET access_count = MAX(access_count, ?1),
             last_accessed_at = CASE
                 WHEN last_accessed_at IS NULL THEN ?2
                 WHEN ?2 IS NULL THEN last_accessed_at
                 WHEN datetime(?2) > datetime(last_accessed_at) THEN ?2
                 ELSE last_accessed_at
             END
         WHERE id = ?3",
        params![
            incoming.access_count,
            incoming.last_accessed_at,
            existing.id
        ],
    )?;
    Ok(())
}

fn add_workflow_review_record(
    conn: &Connection,
    source_db: &Path,
    memory: &Memory,
    err: &anyhow::Error,
) -> Result<()> {
    let context = serde_json::to_string(&json!({
        "kind": "workflow_validation_failed",
        "source_db": source_db.display().to_string(),
        "error": err.to_string(),
        "incoming": {
            "id": &memory.id,
            "name": &memory.name,
            "type": &memory.r#type,
            "description": &memory.description,
            "content": &memory.content,
            "tags": &memory.tags,
            "scope": &memory.scope,
            "source": &memory.source,
            "confidence": &memory.confidence,
            "version": memory.version
        },
        "review": {
            "action": "fix_or_reject_before_import",
            "reason": "workflow memories must be valid before merge can import or update them"
        }
    }))?;
    add_ambiguity_record(
        conn,
        &format!("merge workflow review:{}:{}", memory.scope, memory.name),
        std::slice::from_ref(&memory.id),
        Some(&context),
    )?;
    Ok(())
}
