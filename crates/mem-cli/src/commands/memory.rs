use super::*;

pub(crate) fn cmd_save(app: &App, args: SaveArgs) -> Result<()> {
    let result = save_memory(app, args)?;
    let is_similar = result
        .get("status")
        .and_then(Value::as_str)
        .map(|status| status == "similar_found")
        .unwrap_or(false);
    if is_similar {
        print_json_pretty(&result)?;
    } else {
        print_json(&result)?;
    }
    Ok(())
}

pub(crate) fn save_memory(app: &App, mut args: SaveArgs) -> Result<Value> {
    app.ensure_schema()?;
    validate_tags(&args.tags)?;
    let raw_content = required_content(args.content.take(), args.content_file.as_deref())?;
    workflow_core::validate_memory(
        &args.r#type,
        &raw_content,
        &args.tags,
        &args.scope,
        args.no_validate_workflow,
    )?;

    let conn = app.conn()?;
    if let Some(existing) = memory_by_name(&conn, &args.name)? {
        let content = strip_secrets(&raw_content)?;
        if args.force {
            if source_priority(&args.source) < source_priority(&existing.source) {
                return Ok(json!({
                    "status": "rejected",
                    "reason": "lower_trust_source_cannot_overwrite",
                    "existing": existing,
                    "new_source": args.source
                }));
            }
            let now = now();
            let description = args
                .description
                .or(args.why)
                .or(existing.description.clone());
            let confidence = args
                .confidence
                .unwrap_or_else(|| confidence_for_source(&args.source).to_string());
            with_transaction(&conn, |conn| {
                conn.execute(
                    "UPDATE memories
                     SET type = ?1, description = ?2, content = ?3, tags = ?4, scope = ?5,
                         source = ?6, confidence = ?7, protected = ?8, updated_at = ?9,
                         expires_at = ?10, version = version + 1
                     WHERE id = ?11",
                    params![
                        args.r#type,
                        description,
                        content,
                        args.tags,
                        args.scope,
                        args.source,
                        confidence,
                        args.source == "manual",
                        now,
                        args.expires_at,
                        existing.id
                    ],
                )?;
                log_change(
                    conn,
                    &existing.id,
                    "update",
                    existing.content.as_deref(),
                    Some(&content),
                    &args.source,
                )?;
                Ok(())
            })?;
            memory_index::upsert_or_mark_stale(app, &conn, &existing.id)?;
            let updated = memory_by_id(&conn, &existing.id)?
                .ok_or_else(|| anyhow!("updated memory missing: {}", existing.id))?;
            return Ok(json!({
                "status": "updated",
                "match_type": "exact_name_force",
                "id": updated.id,
                "version": updated.version
            }));
        }
        return Ok(json!({
            "status": "duplicate_found",
            "match_type": "exact_name",
            "existing": existing,
            "new_content": content
        }));
    }

    let content = strip_secrets(&raw_content)?;
    if !args.force {
        memory_index::repair_stale(app)?;
        let candidates = similar_candidates(app, &conn, &content, 5)?;
        if !candidates.is_empty() {
            return Ok(json!({
                "status": "similar_found",
                "match_type": "bm25_lindera",
                "candidates": candidates,
                "new_content": content
            }));
        }
    }

    let id = unique_memory_id(&conn, &slugify(&args.name))?;
    let now = now();
    let confidence = args
        .confidence
        .unwrap_or_else(|| confidence_for_source(&args.source).to_string());
    let protected = args.source == "manual";
    let description = args.description.or(args.why);

    with_transaction(&conn, |conn| {
        conn.execute(
            "INSERT INTO memories
            (id, type, name, description, content, tags, scope, source, confidence, protected, created_at, updated_at, expires_at)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?11, ?12)",
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
                now,
                args.expires_at
            ],
        )
        .context("insert memory")?;
        log_change(conn, &id, "save", None, Some(&content), &args.source)?;
        Ok(())
    })?;
    memory_index::upsert_or_mark_stale(app, &conn, &id)?;

    let mut result = json!({"status": "saved", "id": id, "version": 1});
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

/// Like `save_memory` but skips all index operations (no upsert, no repair_stale).
/// Returns `(json_result, Option<saved_id>)` — `Some(id)` when the record was saved or updated,
/// `None` for duplicates and similar-found results that produce no new/changed record.
pub(crate) fn save_memory_no_index(
    app: &App,
    mut args: SaveArgs,
) -> Result<(Value, Option<String>)> {
    app.ensure_schema()?;
    validate_tags(&args.tags)?;
    let raw_content = required_content(args.content.take(), args.content_file.as_deref())?;
    workflow_core::validate_memory(
        &args.r#type,
        &raw_content,
        &args.tags,
        &args.scope,
        args.no_validate_workflow,
    )?;

    let conn = app.conn()?;
    if let Some(existing) = memory_by_name(&conn, &args.name)? {
        let content = strip_secrets(&raw_content)?;
        if args.force {
            if source_priority(&args.source) < source_priority(&existing.source) {
                return Ok((
                    json!({
                        "status": "rejected",
                        "reason": "lower_trust_source_cannot_overwrite",
                        "existing": existing,
                        "new_source": args.source
                    }),
                    None,
                ));
            }
            let now = now();
            let description = args
                .description
                .or(args.why)
                .or(existing.description.clone());
            let confidence = args
                .confidence
                .unwrap_or_else(|| confidence_for_source(&args.source).to_string());
            with_transaction(&conn, |conn| {
                conn.execute(
                    "UPDATE memories
                     SET type = ?1, description = ?2, content = ?3, tags = ?4, scope = ?5,
                         source = ?6, confidence = ?7, protected = ?8, updated_at = ?9,
                         expires_at = ?10, version = version + 1
                     WHERE id = ?11",
                    params![
                        args.r#type,
                        description,
                        content,
                        args.tags,
                        args.scope,
                        args.source,
                        confidence,
                        args.source == "manual",
                        now,
                        args.expires_at,
                        existing.id
                    ],
                )?;
                log_change(
                    conn,
                    &existing.id,
                    "update",
                    existing.content.as_deref(),
                    Some(&content),
                    &args.source,
                )?;
                Ok(())
            })?;
            let updated = memory_by_id(&conn, &existing.id)?
                .ok_or_else(|| anyhow!("updated memory missing: {}", existing.id))?;
            return Ok((
                json!({
                    "status": "updated",
                    "match_type": "exact_name_force",
                    "id": updated.id,
                    "version": updated.version
                }),
                Some(existing.id),
            ));
        }
        return Ok((
            json!({
                "status": "duplicate_found",
                "match_type": "exact_name",
                "existing": existing,
                "new_content": content
            }),
            None,
        ));
    }

    let content = strip_secrets(&raw_content)?;
    // Note: no repair_stale or similar_candidates check — caller manages index.

    let id = unique_memory_id(&conn, &slugify(&args.name))?;
    let now = now();
    let confidence = args
        .confidence
        .unwrap_or_else(|| confidence_for_source(&args.source).to_string());
    let protected = args.source == "manual";
    let description = args.description.or(args.why);

    with_transaction(&conn, |conn| {
        conn.execute(
            "INSERT INTO memories
            (id, type, name, description, content, tags, scope, source, confidence, protected, created_at, updated_at, expires_at)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?11, ?12)",
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
                now,
                args.expires_at
            ],
        )
        .context("insert memory")?;
        log_change(conn, &id, "save", None, Some(&content), &args.source)?;
        Ok(())
    })?;

    Ok((json!({"status": "saved", "id": id, "version": 1}), Some(id)))
}

pub(crate) fn cmd_update(app: &App, args: UpdateArgs) -> Result<()> {
    app.ensure_schema()?;
    let conn = app.conn()?;
    let old = memory_by_name(&conn, &args.name)?
        .ok_or_else(|| anyhow!("memory not found: {}", args.name))?;
    if let Some(expected) = args.expected_version {
        if let Some(conflict) = version_conflict(&old, expected) {
            print_json(&conflict)?;
            return Ok(());
        }
    }
    if source_priority(&args.source) < source_priority(&old.source) {
        print_json(&json!({
                "status": "rejected",
                "reason": "lower_trust_source_cannot_update",
                "existing_source": old.source,
                "new_source": args.source,
                "id": old.id
        }))?;
        return Ok(());
    }
    let new_content = match optional_content(args.content, args.content_file.as_deref())? {
        Some(content) => Some(strip_secrets(&content)?),
        None => old.content.clone(),
    };
    let description = args.description.or(old.description.clone());
    let tags = match args.add_tags {
        Some(add) => merge_tags(&old.tags, &add)?,
        None => old.tags.clone(),
    };
    workflow_core::validate_memory(
        &old.r#type,
        new_content.as_deref().unwrap_or_default(),
        &tags,
        &old.scope,
        args.no_validate_workflow,
    )?;
    let now = now();

    with_transaction(&conn, |conn| {
        conn.execute(
            "UPDATE memories
            SET description = ?1, content = ?2, tags = ?3, updated_at = ?4, version = version + 1
            WHERE id = ?5",
            params![description, new_content, tags, now, old.id],
        )?;
        log_change(
            conn,
            &old.id,
            "update",
            old.content.as_deref(),
            new_content.as_deref(),
            &args.source,
        )?;
        Ok(())
    })?;
    memory_index::upsert_or_mark_stale(app, &conn, &old.id)?;

    let updated = memory_by_id(&conn, &old.id)?
        .ok_or_else(|| anyhow!("updated memory missing: {}", old.id))?;
    print_json(&json!({"status": "updated", "id": updated.id, "version": updated.version}))?;
    Ok(())
}

pub(crate) fn cmd_supersede(app: &App, args: SupersedeArgs) -> Result<()> {
    app.ensure_schema()?;
    let conn = app.conn()?;
    let old = memory_by_name(&conn, &args.old_name)?
        .ok_or_else(|| anyhow!("memory not found: {}", args.old_name))?;
    if let Some(expected) = args.expected_version {
        if let Some(conflict) = version_conflict(&old, expected) {
            print_json(&conflict)?;
            return Ok(());
        }
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
    let new_id = unique_memory_id(&conn, &slugify(&args.new_name))?;
    let now = now();
    let raw_content = required_content(args.content, args.content_file.as_deref())?;
    workflow_core::validate_memory(
        &old.r#type,
        &raw_content,
        &old.tags,
        &old.scope,
        args.no_validate_workflow,
    )?;
    let content = strip_secrets(&raw_content)?;
    let confidence = confidence_for_source(&args.source);
    let protected = args.source == "manual";

    with_transaction(&conn, |conn| {
        conn.execute(
            "INSERT INTO memories
            (id, type, name, description, content, tags, scope, source, confidence, protected, created_at, updated_at)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?11)",
            params![
                new_id,
                old.r#type,
                args.new_name,
                args.description.or(old.description),
                content,
                old.tags,
                old.scope,
                args.source,
                confidence,
                protected,
                now
            ],
        )?;
        conn.execute(
            "UPDATE memories SET valid_until = ?1, superseded_by = ?2, updated_at = ?1 WHERE id = ?3",
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
        Ok(())
    })?;
    memory_index::upsert_or_mark_stale(app, &conn, &new_id)?;
    memory_index::reindex_or_mark_stale(app, "rebuild index after supersede")?;

    print_json(&json!({"status": "superseded", "old_id": old.id, "new_id": new_id}))?;
    Ok(())
}

pub(crate) fn cmd_delete(app: &App, args: DeleteArgs) -> Result<()> {
    app.ensure_schema()?;
    let conn = app.conn()?;
    let old = memory_by_name(&conn, &args.name)?
        .ok_or_else(|| anyhow!("memory not found: {}", args.name))?;
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
            Ok(())
        })?;
        memory_index::reindex_or_mark_stale(app, "rebuild index after delete")?;
        print_json(&json!({"status": "deleted", "mode": "hard", "id": old.id}))?;
    } else {
        let now = now();
        with_transaction(&conn, |conn| {
            conn.execute(
                "UPDATE memories SET valid_until = ?1, updated_at = ?1 WHERE id = ?2",
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
            Ok(())
        })?;
        memory_index::reindex_or_mark_stale(app, "rebuild index after delete")?;
        print_json(&json!({"status": "deleted", "mode": "soft", "id": old.id}))?;
    }
    Ok(())
}

fn similar_candidates(
    app: &App,
    conn: &Connection,
    content: &str,
    limit: usize,
) -> Result<Vec<Value>> {
    let ids = memory_index::search_ids(app, content, false, false, 25, None, None)?;
    let mut candidates = Vec::new();
    for id in ids {
        let Some(memory) = memory_by_id(conn, &id)? else {
            continue;
        };
        if memory.valid_until.is_some() {
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
