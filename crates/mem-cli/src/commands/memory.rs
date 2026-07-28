use std::collections::BTreeSet;

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

/// Mechanical quality checks only; warnings never block a save. The goal is
/// to hold the memory-quality rules (one fact, absolute dates, tags) without
/// relying on the calling agent to remember them.
fn lint_memory(r#type: &str, name: &str, content: &str, tags: &str) -> Vec<Value> {
    const RELATIVE_DATE_WORDS: &[&str] = &[
        "today",
        "yesterday",
        "tomorrow",
        "last week",
        "next week",
        "this week",
        "recently",
        "currently",
        "今天",
        "昨天",
        "明天",
        "上週",
        "上周",
        "下週",
        "下周",
        "本週",
        "本周",
        "最近",
        "目前",
    ];
    const VAGUE_NAMES: &[&str] = &["note", "notes", "misc", "temp", "todo", "memo", "important"];

    let mut warnings = Vec::new();
    if tags.trim() == "[]" && r#type != "reference" {
        warnings.push(json!({
            "code": "no_tags",
            "hint": "add 2-6 `type:value` tags so retrieval and retros can filter this memory"
        }));
    }
    if r#type != "workflow" && content.chars().count() > 1200 {
        warnings.push(json!({
            "code": "content_long",
            "hint": "content exceeds 1200 chars; split into one fact per memory"
        }));
    }
    if r#type != "workflow" {
        let lowered = content.to_lowercase();
        if let Some(word) = RELATIVE_DATE_WORDS
            .iter()
            .find(|word| lowered.contains(*word))
        {
            warnings.push(json!({
                "code": "relative_date_language",
                "hint": format!("content contains '{word}'; convert relative dates to absolute dates")
            }));
        }
    }
    if r#type != "workflow" {
        let extracted = extract_claims(content);
        if extracted.claims.iter().any(|claim| !claim.backticked) {
            warnings.push(json!({
                "code": "claims_outside_backticks",
                "hint": "content mentions paths outside backticks; wrap them in `...` so `mem reconcile` can verify them"
            }));
        }
    }
    let lowered_name = name.to_lowercase();
    if name.chars().count() < 3 || VAGUE_NAMES.contains(&lowered_name.as_str()) {
        warnings.push(json!({
            "code": "vague_name",
            "hint": "use a short, specific snake_case name that will stay stable"
        }));
    }
    warnings
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

fn resolve_mutation_memory(
    conn: &Connection,
    reference: &str,
    scope_value: &str,
) -> Result<Memory> {
    let scopes = if scope_value == "auto" {
        scope::detect_scope_set()?
    } else {
        scope::validate_scope(scope_value)?;
        vec![scope_value.to_string()]
    };
    let scope_refs = scopes.iter().map(String::as_str).collect::<Vec<_>>();
    let id = resolve_memory_ref_in_scopes(conn, reference, Some(&scope_refs))?;
    memory_by_id(conn, &id)?.ok_or_else(|| anyhow!("memory not found: {reference}"))
}

fn update_tag_set(
    current: &str,
    set: Option<&str>,
    add: Option<&str>,
    remove: Option<&str>,
) -> Result<String> {
    let base = set.unwrap_or(current);
    let mut tags = parse_string_array(base)?
        .into_iter()
        .collect::<BTreeSet<_>>();
    if let Some(add) = add {
        tags.extend(parse_string_array(add)?);
    }
    if let Some(remove) = remove {
        for tag in parse_string_array(remove)? {
            tags.remove(&tag);
        }
    }
    serde_json::to_string(&tags.into_iter().collect::<Vec<_>>()).map_err(Into::into)
}

pub(crate) fn cmd_update(app: &App, args: UpdateArgs) -> Result<()> {
    app.require_schema()?;
    let conn = app.conn()?;
    let old = resolve_mutation_memory(&conn, &args.name, &args.scope)?;
    if let Some(expected) = args.expected_version {
        if let Some(conflict) = version_conflict(&old, expected) {
            print_json(&conflict)?;
            return Ok(());
        }
    }

    let update_source = args.source.as_deref().unwrap_or(&old.source).to_string();
    if update_source == "manual" && !args.user_confirmed {
        bail!("source=manual requires --user-confirmed");
    }
    if source_priority(&update_source) < source_priority(&old.source) {
        print_json(&json!({
            "status": "rejected",
            "reason": "lower_trust_source_cannot_update",
            "existing_source": old.source,
            "new_source": update_source,
            "id": old.id
        }))?;
        return Ok(());
    }

    let new_type = args.r#type.as_deref().unwrap_or(&old.r#type).to_string();
    let new_scope = match args.set_scope.as_deref() {
        Some(scope) => scope::resolve_write_scope(scope)?,
        None => old.scope.clone(),
    };
    if let Some(collision) = memory_by_name_in_scope(&conn, &old.name, &new_scope)? {
        if collision.id != old.id {
            bail!(
                "memory name already exists in destination scope {}: {}",
                new_scope,
                old.name
            );
        }
    }
    let new_content = match optional_content(args.content, args.content_file.as_deref())? {
        Some(content) => Some(sanitize_secret_field(
            &content,
            "content",
            args.redact_secrets,
        )?),
        None => old.content.clone(),
    };
    let description = if args.clear_description {
        None
    } else {
        match args.description.as_deref() {
            Some(value) => Some(sanitize_secret_field(
                value,
                "description",
                args.redact_secrets,
            )?),
            None => old.description.clone(),
        }
    };
    let set_tags = args
        .set_tags
        .as_deref()
        .map(|value| sanitize_secret_field(value, "set_tags", args.redact_secrets))
        .transpose()?;
    let add_tags = args
        .add_tags
        .as_deref()
        .map(|value| sanitize_secret_field(value, "add_tags", args.redact_secrets))
        .transpose()?;
    let remove_tags = args
        .remove_tags
        .as_deref()
        .map(|value| sanitize_secret_field(value, "remove_tags", args.redact_secrets))
        .transpose()?;
    let tags = update_tag_set(
        &old.tags,
        set_tags.as_deref(),
        add_tags.as_deref(),
        remove_tags.as_deref(),
    )?;
    let expires_at = if args.clear_expires_at {
        None
    } else {
        args.expires_at.clone().or(old.expires_at.clone())
    };
    let confidence = args
        .confidence
        .as_deref()
        .unwrap_or(&old.confidence)
        .to_string();
    validate_memory_resource_limits(
        &old.name,
        description.as_deref(),
        new_content.as_deref().unwrap_or_default(),
        &tags,
        &new_scope,
        None,
    )?;
    workflow_core::validate_memory(
        &new_type,
        new_content.as_deref().unwrap_or_default(),
        &tags,
        &new_scope,
        args.no_validate_workflow,
    )?;
    let now = now();
    let user_confirmed_at = (update_source == "manual").then(|| now.clone());

    with_transaction(&conn, |conn| {
        conn.execute(
            "UPDATE memories
             SET type = ?1, description = ?2, content = ?3, tags = ?4, scope = ?5,
                 source = ?6, confidence = ?7, protected = ?8, expires_at = ?9,
                 origin = 'direct', origin_ref = NULL,
                 user_confirmed_at = COALESCE(?10, user_confirmed_at),
                 updated_at = ?11, version = version + 1
             WHERE id = ?12",
            params![
                new_type,
                description,
                new_content,
                tags,
                new_scope,
                update_source,
                confidence,
                update_source == "manual",
                expires_at,
                user_confirmed_at,
                now,
                old.id
            ],
        )?;
        log_change(
            conn,
            &old.id,
            "update",
            old.content.as_deref(),
            new_content.as_deref(),
            &update_source,
        )?;
        mem_core::graph::set_graph_dirty(conn, true)
    })?;
    let updated = memory_by_id(&conn, &old.id)?
        .ok_or_else(|| anyhow!("updated memory missing: {}", old.id))?;
    finish_committed_index_write(
        memory_index::upsert_or_mark_stale(app, &conn, &old.id),
        "memory update",
        json!({"memory_id": old.id, "version": updated.version}),
    )?;
    let warnings = lint_memory(
        &updated.r#type,
        &updated.name,
        updated.content.as_deref().unwrap_or_default(),
        &updated.tags,
    );
    print_write_json(
        app,
        json!({
            "status": "updated",
            "id": updated.id,
            "scope": updated.scope,
            "version": updated.version,
            "warnings": warnings
        }),
    )?;
    Ok(())
}

pub(crate) fn cmd_supersede(app: &App, args: SupersedeArgs) -> Result<()> {
    app.require_schema()?;
    let conn = app.conn()?;
    let old = resolve_mutation_memory(&conn, &args.old_name, &args.scope)?;
    if let Some(expected) = args.expected_version {
        if let Some(conflict) = version_conflict(&old, expected) {
            print_json(&conflict)?;
            return Ok(());
        }
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
    if let Some(expected) = args.expected_version {
        if let Some(conflict) = version_conflict(&old, expected) {
            print_json(&conflict)?;
            return Ok(());
        }
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

fn similar_candidates(
    app: &App,
    conn: &Connection,
    content: &str,
    scope: &str,
    limit: usize,
) -> Result<Vec<Value>> {
    let ids = memory_index::search_ids(
        app,
        content,
        false,
        false,
        25,
        memory_index::SearchFilters::default(),
        true,
    )?;
    let mut candidates = Vec::new();
    for id in ids {
        let Some(memory) = memory_by_id(conn, &id)? else {
            continue;
        };
        if !memory_is_active(&memory) || memory.scope != scope {
            continue;
        }
        let score = content_similarity(content, memory.content.as_deref().unwrap_or_default());
        if score >= 0.55 {
            candidates.push(json!({
                "id": memory.id,
                "name": memory.name,
                "content": memory.content,
                "score": score
            }));
        }
    }
    candidates.sort_by(|a, b| {
        let a_score = a.get("score").and_then(Value::as_f64).unwrap_or_default();
        let b_score = b.get("score").and_then(Value::as_f64).unwrap_or_default();
        b_score
            .partial_cmp(&a_score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    candidates.truncate(limit);
    Ok(candidates)
}
