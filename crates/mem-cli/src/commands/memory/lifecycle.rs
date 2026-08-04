use super::update::resolve_mutation_memory;
use super::*;

pub(crate) fn cmd_supersede(app: &App, args: SupersedeArgs) -> Result<()> {
    app.require_schema()?;
    let conn = app.conn()?;
    let old = resolve_mutation_memory(&conn, &args.old_name, &args.scope)?;
    if let Some(expected) = args.expected_version
        && let Some(conflict) = version_conflict(&old, expected)
    {
        print_json(&conflict)?;
        return Ok(());
    }
    if args.source == "manual" && !args.user_confirmed {
        bail!("source=manual requires --user-confirmed");
    }
    if source_priority(&args.source) < source_priority(&old.source) {
        print_json(&json!({
            "status": "rejected",
            "reason": "lower_trust_source_cannot_supersede",
            "existing_source": old.source,
            "new_source": args.source,
            "id": old.id
        }))?;
        return Ok(());
    }
    let new_scope = match args.new_scope.as_deref() {
        Some(scope) => scope::resolve_write_scope(scope)?,
        None => old.scope.clone(),
    };
    let new_name = sanitize_secret_field(&args.new_name, "name", args.redact_secrets)?;
    if memory_by_name_in_scope(&conn, &new_name, &new_scope)?.is_some() {
        bail!("memory already exists in scope {new_scope}: {new_name}");
    }
    let new_id = unique_memory_id(&conn, &slugify(&new_name))?;
    let now = now();
    let raw_content = required_content(args.content, args.content_file.as_deref())?;
    let content = sanitize_secret_field(&raw_content, "content", args.redact_secrets)?;
    let description = match args.description.as_deref() {
        Some(value) => Some(sanitize_secret_field(
            value,
            "description",
            args.redact_secrets,
        )?),
        None => old.description.clone(),
    };
    validate_memory_resource_limits(
        &new_name,
        description.as_deref(),
        &content,
        &old.tags,
        &new_scope,
        None,
    )?;
    workflow_core::validate_memory(
        &old.r#type,
        &content,
        &old.tags,
        &new_scope,
        args.no_validate_workflow,
    )?;
    let confidence = confidence_for_source(&args.source);
    let protected = args.source == "manual";
    let user_confirmed_at = protected.then(|| now.clone());

    with_transaction(&conn, |conn| {
        conn.execute(
            "INSERT INTO memories
            (id, type, name, description, content, tags, scope, source, confidence, protected,
             created_at, updated_at, origin, user_confirmed_at)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?11, 'direct', ?12)",
            params![
                new_id,
                old.r#type,
                new_name,
                description,
                content,
                old.tags,
                new_scope,
                args.source,
                confidence,
                protected,
                now,
                user_confirmed_at
            ],
        )?;
        conn.execute(
            "UPDATE memories
             SET valid_until = ?1, superseded_by = ?2, updated_at = ?1,
                 version = version + 1
             WHERE id = ?3",
            params![now, new_id, old.id],
        )?;
        log_change(
            conn,
            &old.id,
            "supersede",
            old.content.as_deref(),
            Some(&content),
            &args.source,
        )?;
        mem_core::graph::set_graph_dirty(conn, true)
    })?;
    let index_details = json!({
        "old_memory_id": old.id,
        "new_memory_id": new_id,
        "new_version": 1
    });
    finish_committed_index_write(
        memory_index::upsert_or_mark_stale(app, &conn, &new_id),
        "memory supersede",
        index_details.clone(),
    )?;
    finish_committed_index_write(
        memory_index::reindex_or_mark_stale(app, "rebuild index after supersede"),
        "memory supersede",
        index_details,
    )?;

    print_write_json(
        app,
        json!({
            "status": "superseded",
            "old_id": old.id,
            "new_id": new_id,
            "scope": new_scope
        }),
    )?;
    Ok(())
}

pub(crate) fn cmd_delete(app: &App, args: DeleteArgs) -> Result<()> {
    app.require_schema()?;
    let conn = app.conn()?;
    let old = resolve_mutation_memory(&conn, &args.name, &args.scope)?;
    if args.source == "manual" && !args.user_confirmed {
        bail!("source=manual requires --user-confirmed");
    }
    if source_priority(&args.source) < source_priority(&old.source) {
        print_json(&json!({
            "status": "rejected",
            "reason": "lower_trust_source_cannot_delete",
            "existing_source": old.source,
            "new_source": args.source,
            "id": old.id
        }))?;
        return Ok(());
    }
    if let Some(expected) = args.expected_version
        && let Some(conflict) = version_conflict(&old, expected)
    {
        print_json(&conflict)?;
        return Ok(());
    }
    if old.protected && !args.force {
        print_json(
            &json!({"status": "rejected", "reason": "protected_memory_requires_force", "id": old.id}),
        )?;
        return Ok(());
    }

    if args.hard {
        with_transaction(&conn, |conn| {
            conn.execute("DELETE FROM memories WHERE id = ?1", params![old.id])?;
            log_change(
                conn,
                &old.id,
                "delete",
                old.content.as_deref(),
                None,
                &args.source,
            )?;
            mem_core::graph::set_graph_dirty(conn, true)
        })?;
        finish_committed_index_write(
            memory_index::reindex_or_mark_stale(app, "rebuild index after delete"),
            "hard memory delete",
            json!({"memory_id": old.id, "mode": "hard"}),
        )?;
        print_write_json(
            app,
            json!({"status": "deleted", "mode": "hard", "id": old.id}),
        )?;
    } else {
        let now = now();
        with_transaction(&conn, |conn| {
            conn.execute(
                "UPDATE memories
                 SET valid_until = ?1, updated_at = ?1, version = version + 1
                 WHERE id = ?2",
                params![now, old.id],
            )?;
            log_change(
                conn,
                &old.id,
                "delete",
                old.content.as_deref(),
                None,
                &args.source,
            )?;
            mem_core::graph::set_graph_dirty(conn, true)
        })?;
        finish_committed_index_write(
            memory_index::reindex_or_mark_stale(app, "rebuild index after delete"),
            "soft memory delete",
            json!({
                "memory_id": old.id,
                "mode": "soft",
                "version": old.version + 1
            }),
        )?;
        print_write_json(
            app,
            json!({"status": "deleted", "mode": "soft", "id": old.id}),
        )?;
    }
    Ok(())
}
