use super::lint::lint_memory;
use super::similarity::similar_candidates;
use super::*;

pub(crate) fn cmd_save(app: &App, args: SaveArgs) -> Result<()> {
    let result = save_memory(app, args)?;
    let is_similar = result
        .get("status")
        .and_then(Value::as_str)
        .map(|status| status == "similar_found")
        .unwrap_or(false);
    if is_similar {
        print_write_json_pretty(app, result)?;
    } else {
        print_write_json(app, result)?;
    }
    Ok(())
}

fn prepare_save_args(mut args: SaveArgs) -> Result<(SaveArgs, String)> {
    args.scope = scope::resolve_write_scope(&args.scope)?;
    if args.source == "manual" && !args.user_confirmed {
        bail!("source=manual requires --user-confirmed");
    }
    args.name = sanitize_secret_field(&args.name, "name", args.redact_secrets)?;
    args.description = args
        .description
        .as_deref()
        .map(|value| sanitize_secret_field(value, "description", args.redact_secrets))
        .transpose()?;
    args.why = args
        .why
        .as_deref()
        .map(|value| sanitize_secret_field(value, "why", args.redact_secrets))
        .transpose()?;
    args.tags = sanitize_secret_field(&args.tags, "tags", args.redact_secrets)?;
    validate_tags(&args.tags)?;
    let raw_content = required_content(args.content.take(), args.content_file.as_deref())?;
    let content = sanitize_secret_field(&raw_content, "content", args.redact_secrets)?;
    validate_memory_resource_limits(
        &args.name,
        args.description.as_deref(),
        &content,
        &args.tags,
        &args.scope,
        args.why.as_deref(),
    )?;
    workflow_core::validate_memory(
        &args.r#type,
        &content,
        &args.tags,
        &args.scope,
        args.no_validate_workflow,
    )?;
    Ok((args, content))
}

pub(crate) fn save_memory(app: &App, args: SaveArgs) -> Result<Value> {
    app.require_schema()?;
    let (args, content) = prepare_save_args(args)?;
    let conn = app.conn()?;

    if let Some(existing) = memory_by_name_in_scope(&conn, &args.name, &args.scope)? {
        if !existing_save_will_write(&args, &existing) {
            return Ok(persist_existing_memory(&conn, &args, &content, &existing)?.result);
        }
        let persisted = with_transaction(&conn, |conn| {
            let persisted = persist_existing_memory(conn, &args, &content, &existing)?;
            if persisted.changed_id.is_some() {
                mem_core::graph::set_graph_dirty(conn, true)?;
            }
            Ok(persisted)
        })?;
        if let Some(id) = persisted.changed_id.as_deref() {
            finish_committed_index_write(
                memory_index::upsert_or_mark_stale(app, &conn, id),
                "memory update",
                json!({
                    "memory_id": id,
                    "version": persisted.result.get("version").and_then(Value::as_i64)
                }),
            )?;
        }
        return Ok(persisted.result);
    }

    if !args.force {
        memory_index::repair_stale(app)?;
        let candidates = similar_candidates(app, &conn, &content, &args.scope, 5)?;
        if !candidates.is_empty() {
            return Ok(json!({
                "status": "similar_found",
                "match_type": "bm25_lindera",
                "candidates": candidates,
                "new_content": content
            }));
        }
    }

    let persisted = with_transaction(&conn, |conn| {
        let persisted = persist_new_memory(conn, &args, &content)?;
        mem_core::graph::set_graph_dirty(conn, true)?;
        Ok(persisted)
    })?;
    let id = persisted
        .changed_id
        .as_deref()
        .ok_or_else(|| anyhow!("new save did not return a changed memory id"))?;
    finish_committed_index_write(
        memory_index::upsert_or_mark_stale(app, &conn, id),
        "memory save",
        json!({"memory_id": id, "version": 1}),
    )?;

    let mut result = persisted.result;
    let warnings = lint_memory(&args.r#type, &args.name, &content, &args.tags);
    if !warnings.is_empty() {
        result["warnings"] = json!(warnings);
    }
    Ok(result)
}

struct PersistedSave {
    result: Value,
    changed_id: Option<String>,
}

fn existing_save_will_write(args: &SaveArgs, existing: &Memory) -> bool {
    args.force && source_priority(&args.source) >= source_priority(&existing.source)
}

fn persist_existing_memory(
    conn: &Connection,
    args: &SaveArgs,
    content: &str,
    existing: &Memory,
) -> Result<PersistedSave> {
    if !args.force {
        return Ok(PersistedSave {
            result: json!({
                "status": "duplicate_found",
                "match_type": "exact_name",
                "existing": existing,
                "new_content": content
            }),
            changed_id: None,
        });
    }
    if source_priority(&args.source) < source_priority(&existing.source) {
        return Ok(PersistedSave {
            result: json!({
                "status": "rejected",
                "reason": "lower_trust_source_cannot_overwrite",
                "existing": existing,
                "new_source": args.source
            }),
            changed_id: None,
        });
    }

    let timestamp = now();
    let user_confirmed_at = (args.source == "manual").then(|| timestamp.clone());
    let description = args
        .description
        .clone()
        .or_else(|| args.why.clone())
        .or_else(|| existing.description.clone());
    let confidence = args
        .confidence
        .clone()
        .unwrap_or_else(|| confidence_for_source(&args.source).to_string());
    conn.execute(
        "UPDATE memories
         SET type = ?1, description = ?2, content = ?3, tags = ?4, scope = ?5,
             source = ?6, confidence = ?7, protected = ?8, updated_at = ?9,
             expires_at = ?10, origin = ?11, origin_ref = ?12,
             user_confirmed_at = COALESCE(?13, user_confirmed_at),
             version = version + 1
         WHERE id = ?14",
        params![
            args.r#type,
            description,
            content,
            args.tags,
            args.scope,
            args.source,
            confidence,
            args.source == "manual",
            timestamp,
            args.expires_at,
            args.origin.as_deref().unwrap_or("direct"),
            args.origin_ref,
            user_confirmed_at,
            existing.id
        ],
    )?;
    log_change(
        conn,
        &existing.id,
        "update",
        existing.content.as_deref(),
        Some(content),
        &args.source,
    )?;
    let updated = memory_by_id(conn, &existing.id)?
        .ok_or_else(|| anyhow!("updated memory missing: {}", existing.id))?;
    Ok(PersistedSave {
        result: json!({
            "status": "updated",
            "match_type": "exact_name_force",
            "id": updated.id,
            "version": updated.version
        }),
        changed_id: Some(existing.id.clone()),
    })
}

fn persist_new_memory(conn: &Connection, args: &SaveArgs, content: &str) -> Result<PersistedSave> {
    let id = unique_memory_id(conn, &slugify(&args.name))?;
    let timestamp = now();
    let confidence = args
        .confidence
        .clone()
        .unwrap_or_else(|| confidence_for_source(&args.source).to_string());
    let protected = args.source == "manual";
    let user_confirmed_at = protected.then(|| timestamp.clone());
    let description = args.description.clone().or_else(|| args.why.clone());
    let origin = args.origin.as_deref().unwrap_or("direct");

    conn.execute(
        "INSERT INTO memories
        (id, type, name, description, content, tags, scope, source, confidence, protected,
         created_at, updated_at, expires_at, origin, origin_ref, user_confirmed_at)
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?11, ?12, ?13, ?14, ?15)",
        params![
            id,
            args.r#type,
            args.name,
            description,
            content,
            args.tags,
            args.scope,
            args.source,
            confidence,
            protected,
            timestamp,
            args.expires_at,
            origin,
            args.origin_ref,
            user_confirmed_at
        ],
    )
    .context("insert memory")?;
    log_change(conn, &id, "save", None, Some(content), &args.source)?;

    Ok(PersistedSave {
        result: json!({"status": "saved", "id": id, "version": 1}),
        changed_id: Some(id),
    })
}

/// Save one imported memory inside a caller-owned transaction or savepoint,
/// skipping similarity and index operations. The caller batches index updates
/// and marks graph materialization dirty once.
pub(crate) fn save_memory_no_index_in_connection(
    conn: &Connection,
    args: SaveArgs,
) -> Result<(Value, Option<String>)> {
    let (args, content) = prepare_save_args(args)?;
    let persisted = match memory_by_name_in_scope(conn, &args.name, &args.scope)? {
        Some(existing) => persist_existing_memory(conn, &args, &content, &existing)?,
        None => persist_new_memory(conn, &args, &content)?,
    };
    Ok((persisted.result, persisted.changed_id))
}
