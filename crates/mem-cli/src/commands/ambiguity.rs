use super::*;

pub(crate) fn cmd_ambiguity(app: &App, command: AmbiguityCommand) -> Result<()> {
    app.require_schema()?;
    match command {
        AmbiguityCommand::Add(args) => {
            let conn = app.conn()?;
            validate_tags(&args.memory_ids)?;
            sanitize_secret_field(&args.memory_ids, "ambiguity memory ids", false)?;
            let memory_ids = parse_string_array(&args.memory_ids)?;
            if memory_ids.len() > 1_000 {
                bail!("ambiguity memory_ids cannot exceed 1000 entries");
            }
            if args.query.len() > 10_000 {
                bail!("ambiguity query exceeds 10000 bytes");
            }
            if args
                .context
                .as_deref()
                .is_some_and(|value| value.len() > 4_194_304)
            {
                bail!("ambiguity context exceeds 4194304 bytes");
            }
            let query = sanitize_secret_field(&args.query, "ambiguity query", args.redact_secrets)?;
            let context = args
                .context
                .as_deref()
                .map(|value| sanitize_secret_field(value, "ambiguity context", args.redact_secrets))
                .transpose()?;
            let id = add_ambiguity_record(&conn, &query, &memory_ids, context.as_deref())?;
            print_json(&json!({"status": "ambiguity_added", "id": id}))?;
        }
        AmbiguityCommand::List(args) => {
            let conn = app.read_conn()?;
            let rows = ambiguity_rows(&conn, args.pending)?;
            print_json_pretty(&rows)?;
        }
        AmbiguityCommand::Resolve(args) => {
            let conn = app.conn()?;
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
            let scopes = if args.scope == "auto" {
                scope::detect_scope_set()?
            } else {
                scope::validate_scope(&args.scope)?;
                vec![args.scope.clone()]
            };
            let scope_refs = scopes.iter().map(String::as_str).collect::<Vec<_>>();
            if let Some(reference) = args.keep.as_deref() {
                sanitize_secret_field(reference, "ambiguity keep reference", false)?;
            }
            if args
                .note
                .as_deref()
                .is_some_and(|value| value.len() > 65_536)
            {
                bail!("ambiguity resolution note exceeds 65536 bytes");
            }
            let note = args
                .note
                .as_deref()
                .map(|value| {
                    sanitize_secret_field(value, "ambiguity resolution note", args.redact_secrets)
                })
                .transpose()?;
            let keep_id = match args.keep.as_deref() {
                Some(reference) => Some(resolve_memory_ref_in_scopes(
                    &conn,
                    reference,
                    Some(&scope_refs),
                )?),
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
                            "UPDATE memories
                             SET valid_until = ?1, updated_at = ?1, version = version + 1
                             WHERE id = ?2",
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
                    "note": &note,
                    "keep": keep_id,
                    "soft_deleted": soft_deleted,
                    "skipped_protected": skipped_protected
                })
                .to_string();
                conn.execute(
                    "UPDATE ambiguities SET resolution = ?1, resolved_at = ?2 WHERE id = ?3",
                    params![resolution, now, args.id],
                )?;
                if !soft_deleted.is_empty() {
                    mem_core::graph::set_graph_dirty(conn, true)?;
                }
                Ok(!soft_deleted.is_empty())
            })?;
            if reindex_needed {
                finish_committed_index_write(
                    memory_index::reindex_or_mark_stale(
                        app,
                        "rebuild index after ambiguity resolution",
                    ),
                    "ambiguity resolution",
                    json!({
                        "ambiguity_id": args.id,
                        "soft_deleted_count": soft_deleted.len()
                    }),
                )?;
            }
            print_json_pretty(&json!({
                "status": "resolved",
                "id": args.id,
                "resolution": {
                    "status": "resolved",
                    "note": &note,
                    "keep": keep_id,
                    "soft_deleted": soft_deleted,
                    "skipped_protected": skipped_protected
                }
            }))?;
        }
    }
    Ok(())
}
