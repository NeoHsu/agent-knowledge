use super::*;

pub(crate) fn cmd_ambiguity(app: &App, command: AmbiguityCommand) -> Result<()> {
    app.ensure_schema()?;
    let conn = app.conn()?;
    match command {
        AmbiguityCommand::Add(args) => {
            validate_tags(&args.memory_ids)?;
            let memory_ids = parse_string_array(&args.memory_ids)?;
            add_ambiguity_record(&conn, &args.query, &memory_ids, args.context.as_deref())?;
            print_json(&json!({"status": "ambiguity_added", "id": conn.last_insert_rowid()}))?;
        }
        AmbiguityCommand::List(args) => {
            let rows = ambiguity_rows(&conn, args.pending)?;
            print_json_pretty(&rows)?;
        }
        AmbiguityCommand::Resolve(args) => {
            let now = now();
            let ambiguity = ambiguity_by_id(&conn, args.id)?
                .ok_or_else(|| anyhow!("ambiguity not found: {}", args.id))?;
            let raw_memory_ids = ambiguity
                .get("memory_ids")
                .and_then(Value::as_str)
                .unwrap_or("[]");
            let memory_ids = parse_string_array(raw_memory_ids)?;
            let mut soft_deleted = Vec::new();
            let mut skipped_protected = Vec::new();
            let keep_id = match args.keep.as_deref() {
                Some(reference) => Some(resolve_memory_ref(&conn, reference)?),
                None => None,
            };
            let reindex_needed = with_transaction(&conn, |conn| {
                if args.soft_delete_others {
                    let keep_id = keep_id
                        .as_deref()
                        .ok_or_else(|| anyhow!("--soft-delete-others requires --keep"))?;
                    for memory_id in memory_ids.iter().filter(|id| id.as_str() != keep_id) {
                        let Some(memory) = memory_by_id(conn, memory_id)? else {
                            continue;
                        };
                        if memory.protected {
                            skipped_protected.push(memory.id);
                            continue;
                        }
                        conn.execute(
                            "UPDATE memories SET valid_until = ?1, updated_at = ?1 WHERE id = ?2",
                            params![now, memory.id],
                        )?;
                        log_change(
                            conn,
                            &memory.id,
                            "delete",
                            memory.content.as_deref(),
                            None,
                            "ambiguity_resolve",
                        )?;
                        soft_deleted.push(memory.id);
                    }
                }

                let resolution = json!({
                    "status": "resolved",
                    "note": args.note,
                    "keep": keep_id,
                    "soft_deleted": soft_deleted,
                    "skipped_protected": skipped_protected
                })
                .to_string();
                conn.execute(
                    "UPDATE ambiguities SET resolution = ?1, resolved_at = ?2 WHERE id = ?3",
                    params![resolution, now, args.id],
                )?;
                Ok(!soft_deleted.is_empty())
            })?;
            if reindex_needed {
                memory_index::reindex_or_mark_stale(
                    app,
                    "rebuild index after ambiguity resolution",
                )?;
            }
            print_json_pretty(&json!({
                "status": "resolved",
                "id": args.id,
                "resolution": {
                    "status": "resolved",
                    "note": args.note,
                    "keep": keep_id,
                    "soft_deleted": soft_deleted,
                    "skipped_protected": skipped_protected
                }
            }))?;
        }
    }
    Ok(())
}
