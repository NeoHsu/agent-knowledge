use super::*;

pub(crate) fn cmd_query(app: &App, args: QueryArgs) -> Result<()> {
    app.init()?;
    if args.semantic {
        println!(
            "{}",
            json!({
                "status": "unsupported",
                "feature": "semantic_query",
                "message": "semantic query interface is reserved; no embedding backend is configured"
            })
        );
        return Ok(());
    }
    let conn = app.conn()?;
    let scope_filter = match args.scope.as_deref() {
        Some("auto") => Some(scope::detect_scope_set()?),
        Some(scope) => Some(vec!["global".to_string(), scope.to_string()]),
        None => None,
    };

    let mut ids = if let Some(query) = args.query.as_deref() {
        memory_index::repair_stale(app)?;
        memory_index::search_ids(
            app,
            query,
            args.fuzzy,
            args.raw_query,
            args.limit.max(DEFAULT_LIMIT),
        )?
    } else {
        Vec::new()
    };

    let mut memories = if args.query.is_some() {
        let mut rows = Vec::new();
        for id in ids.drain(..) {
            if let Some(memory) = memory_by_id(&conn, &id)? {
                rows.push(memory);
            }
        }
        rows
    } else {
        all_memories(&conn)?
    };

    memories.retain(|memory| {
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
        if let Some(scopes) = &scope_filter {
            if !scopes.contains(&memory.scope) {
                return false;
            }
        }
        if args.expired {
            return is_expired(memory.expires_at.as_deref());
        }
        true
    });

    match args.sort {
        SortMode::Relevance => {}
        SortMode::Time => memories.sort_by(|a, b| b.created_at.cmp(&a.created_at)),
        SortMode::AccessCount => {
            memories.sort_by_key(|memory| std::cmp::Reverse(memory.access_count))
        }
    }
    memories.truncate(args.limit);

    if !args.no_touch {
        let now = now();
        for memory in &memories {
            conn.execute(
                "UPDATE memories SET access_count = access_count + 1, last_accessed_at = ?1 WHERE id = ?2",
                params![now, memory.id],
            )?;
        }
    }

    print_json_pretty(&memories)?;
    Ok(())
}
