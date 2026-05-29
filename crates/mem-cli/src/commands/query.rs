use super::*;

pub(crate) fn cmd_query(app: &App, args: QueryArgs) -> Result<()> {
    app.ensure_schema()?;
    // TODO(F2): --semantic is intentionally hidden until an embedding backend
    // is planned and configured. Keep the guard so hidden/manual use fails
    // explicitly instead of silently falling back to full-text search.
    if args.semantic {
        anyhow::bail!(
            "--semantic is not yet implemented; no embedding backend is configured. \
             Remove --semantic to use full-text/fuzzy search instead."
        );
    }
    let conn = app.conn()?;
    let limit = args
        .limit
        .or_else(|| app.config.query_default_limit())
        .unwrap_or(DEFAULT_LIMIT);
    let scope = args
        .scope
        .as_deref()
        .or_else(|| app.config.query_default_scope());
    let scope_filter = match scope {
        Some("auto") => Some(scope::detect_scope_set()?),
        Some(scope) => Some(vec!["global".to_string(), scope.to_string()]),
        None => None,
    };

    let ids = if let Some(query) = args.query.as_deref() {
        memory_index::repair_stale(app)?;
        let search_limit = memory_count(&conn)?.max(limit).max(DEFAULT_LIMIT);
        memory_index::search_ids(
            app,
            query,
            args.fuzzy,
            args.raw_query,
            search_limit,
            args.r#type.as_deref(),
            scope_filter.as_deref(),
        )?
    } else {
        Vec::new()
    };

    // P2: when listing (no query), push filters into SQL to avoid a full table scan.
    let mut memories = if args.query.is_some() {
        let mut by_id = memories_by_ids(&conn, &ids)?;
        let mut rows = ids
            .iter()
            .filter_map(|id| by_id.remove(id))
            .collect::<Vec<_>>();
        // Post-filter search results (IDs come from tantivy which doesn't know about filters).
        rows.retain(|memory| passes_filters(memory, &args, scope_filter.as_deref()));
        rows
    } else {
        // Push all applicable filters into the SQL WHERE clause.
        mem_core::db::list_memories_filtered(
            &conn,
            args.include_superseded,
            args.r#type.as_deref(),
            args.tags.as_deref(),
            scope_filter.as_deref(),
            args.expired,
        )?
    };

    match args.sort {
        SortMode::Relevance => {}
        SortMode::Time => memories.sort_by(|a, b| b.created_at.cmp(&a.created_at)),
        SortMode::AccessCount => {
            memories.sort_by_key(|memory| std::cmp::Reverse(memory.access_count))
        }
    }
    memories.truncate(limit);

    // P4: wrap all access_count updates in a single transaction instead of
    // individual UPDATE statements in a loop.
    if !args.no_touch && !memories.is_empty() {
        let now = now();
        let ids_list: Vec<&str> = memories.iter().map(|m| m.id.as_str()).collect();
        // Build WHERE id IN (?2, ?3, ...) — ?1 is reserved for `now`.
        let sql = format!(
            "UPDATE memories SET access_count = access_count + 1, last_accessed_at = ?1 WHERE id IN ({})",
            (2..=ids_list.len() + 1)
                .map(|i| format!("?{i}"))
                .collect::<Vec<_>>()
                .join(", ")
        );
        let mut stmt = conn.prepare(&sql)?;
        let mut bind_params: Vec<&dyn rusqlite::types::ToSql> = vec![&now];
        for id in &ids_list {
            bind_params.push(id);
        }
        stmt.execute(bind_params.as_slice())?;
    }

    match args.format {
        OutputFormat::Json => print_json_pretty(&memories)?,
        OutputFormat::Table => print_text(render_memory_table(&memories))?,
        OutputFormat::Compact => print_text(render_memory_compact(&memories))?,
    }
    Ok(())
}

fn passes_filters(
    memory: &mem_core::db::Memory,
    args: &QueryArgs,
    scope_filter: Option<&[String]>,
) -> bool {
    if !args.include_superseded && memory.valid_until.is_some() {
        return false;
    }
    if let Some(want_type) = &args.r#type {
        if &memory.r#type != want_type {
            return false;
        }
    }
    if let Some(tag) = &args.tags {
        if !memory_has_tag(&memory.tags, tag) {
            return false;
        }
    }
    if let Some(scopes) = scope_filter {
        if !scopes.contains(&memory.scope) {
            return false;
        }
    }
    if args.expired {
        return is_expired(memory.expires_at.as_deref());
    }
    true
}

fn render_memory_table(memories: &[mem_core::db::Memory]) -> String {
    let rows = memories
        .iter()
        .map(|memory| {
            vec![
                truncate_text(&memory.id, 28),
                truncate_text(&memory.name, 28),
                memory.r#type.clone(),
                truncate_text(&memory.scope, 32),
                memory.confidence.clone(),
                truncate_text(&tags_text(&memory.tags), 36),
                memory.access_count.to_string(),
                truncate_text(&memory.updated_at, 20),
            ]
        })
        .collect::<Vec<_>>();
    render_table(
        &[
            "id",
            "name",
            "type",
            "scope",
            "confidence",
            "tags",
            "access",
            "updated",
        ],
        &rows,
    )
}

fn render_memory_compact(memories: &[mem_core::db::Memory]) -> String {
    let mut output = String::new();
    for memory in memories {
        let tags = tags_text(&memory.tags);
        let suffix = if tags.is_empty() {
            String::new()
        } else {
            format!(" tags={tags}")
        };
        output.push_str(&format!(
            "{} [{}] scope={} confidence={}{}",
            memory.name, memory.r#type, memory.scope, memory.confidence, suffix
        ));
        output.push('\n');
        if let Some(description) = memory.description.as_deref() {
            if !description.trim().is_empty() {
                output.push_str(&format!("  {}\n", truncate_text(description, 120)));
            }
        }
        if let Some(content) = memory.content.as_deref() {
            if !content.trim().is_empty() {
                output.push_str(&format!("  {}\n", truncate_text(content, 160)));
            }
        }
    }
    output
}

fn tags_text(tags: &str) -> String {
    parse_string_array(tags)
        .map(|tags| tags.join(","))
        .unwrap_or_else(|_| tags.to_string())
}
