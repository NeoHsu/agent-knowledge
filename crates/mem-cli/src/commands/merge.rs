use super::*;

pub(crate) fn cmd_merge(app: &App, args: MergeArgs) -> Result<()> {
    app.ensure_schema()?;
    if !args.db.exists() {
        bail!("merge database not found: {}", args.db.display());
    }

    let conn = app.conn()?;
    let theirs = Connection::open(&args.db)
        .with_context(|| format!("open merge database {}", args.db.display()))?;
    let incoming = all_memories(&theirs)?;
    let mut imported = 0;
    let mut identical = 0;
    let mut conflicts = 0;
    let mut trusted_updates = 0;
    let mut rejected_lower_trust = 0;
    let mut regenerated_ids = 0;
    let mut workflow_review_required = 0;
    let mut changed_index_ids = Vec::new();

    with_transaction(&conn, |conn| {
        for mut memory in incoming {
            if let Some(content) = memory.content.take() {
                memory.content = Some(strip_secrets(&content)?);
            }
            if let Err(err) = workflow_core::validate_record_content(&memory) {
                add_workflow_review_record(conn, &args.db, &memory, &err)?;
                workflow_review_required += 1;
                continue;
            }

            if let Some(existing) = memory_by_name(conn, &memory.name)? {
                if normalized_text(existing.content.as_deref().unwrap_or_default())
                    == normalized_text(memory.content.as_deref().unwrap_or_default())
                {
                    identical += 1;
                    continue;
                }

                let incoming_priority = source_priority(&memory.source);
                let existing_priority = source_priority(&existing.source);
                if incoming_priority < existing_priority {
                    rejected_lower_trust += 1;
                    continue;
                }
                if args.prefer_trusted && incoming_priority > existing_priority {
                    update_memory_from_merge(conn, &existing, &memory)?;
                    changed_index_ids.push(existing.id.clone());
                    trusted_updates += 1;
                    continue;
                }

                let context = serde_json::to_string(&json!({
                    "kind": "merge_conflict",
                    "source_db": args.db.display().to_string(),
                    "local": {
                        "id": &existing.id,
                        "name": &existing.name,
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
                    &format!("merge:{}", memory.name),
                    &[existing.id.clone(), memory.id.clone()],
                    Some(&context),
                )?;
                conflicts += 1;
                continue;
            }

            let original_id = memory.id.clone();
            memory.id = unique_memory_id(conn, &memory.id)?;
            if memory.id != original_id {
                regenerated_ids += 1;
            }
            insert_memory_record(conn, &memory)?;
            log_change(
                conn,
                &memory.id,
                "merge",
                None,
                memory.content.as_deref(),
                "merge",
            )?;
            changed_index_ids.push(memory.id.clone());
            imported += 1;
        }
        Ok(())
    })?;

    for id in &changed_index_ids {
        memory_index::upsert_or_mark_stale(app, &conn, id)?;
    }

    print_json_pretty(&json!({
        "status": "merged",
        "imported": imported,
        "identical": identical,
        "conflicts": conflicts,
        "trusted_updates": trusted_updates,
        "rejected_lower_trust": rejected_lower_trust,
        "workflow_review_required": workflow_review_required,
        "regenerated_ids": regenerated_ids
    }))?;
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
        &format!("merge workflow review:{}", memory.name),
        std::slice::from_ref(&memory.id),
        Some(&context),
    )
}
