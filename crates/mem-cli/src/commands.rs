use std::fs;
use std::path::Path;

use anyhow::{anyhow, bail, Context, Result};
use chrono::{Duration, Utc};
use rusqlite::{params, Connection};
use serde::Serialize;
use serde_json::{json, Value};

use crate::args::*;
use mem_core::app::App;
use mem_core::db::*;
use mem_core::index as memory_index;
use mem_core::scope;
use mem_core::util::*;
use mem_core::workflow;

pub(crate) fn print_json<T: Serialize + ?Sized>(value: &T) -> Result<()> {
    println!("{}", serde_json::to_string(value)?);
    Ok(())
}

pub(crate) fn print_json_pretty<T: Serialize + ?Sized>(value: &T) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

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
    app.init()?;
    validate_tags(&args.tags)?;
    let raw_content = required_content(args.content.take(), args.content_file.as_deref())?;
    workflow::validate_memory(
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
            let updated = memory_by_id(&conn, &existing.id)?.expect("updated memory exists");
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

    Ok(json!({"status": "saved", "id": id, "version": 1}))
}

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

pub(crate) fn cmd_update(app: &App, args: UpdateArgs) -> Result<()> {
    app.init()?;
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
        println!(
            "{}",
            json!({
                "status": "rejected",
                "reason": "lower_trust_source_cannot_update",
                "existing_source": old.source,
                "new_source": args.source,
                "id": old.id
            })
        );
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
    workflow::validate_memory(
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

    let updated = memory_by_id(&conn, &old.id)?.expect("updated memory exists");
    println!(
        "{}",
        json!({"status": "updated", "id": updated.id, "version": updated.version})
    );
    Ok(())
}

pub(crate) fn cmd_supersede(app: &App, args: SupersedeArgs) -> Result<()> {
    app.init()?;
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
        println!(
            "{}",
            json!({
                "status": "rejected",
                "reason": "lower_trust_source_cannot_supersede",
                "existing_source": old.source,
                "new_source": args.source,
                "id": old.id
            })
        );
        return Ok(());
    }
    let new_id = unique_memory_id(&conn, &slugify(&args.new_name))?;
    let now = now();
    let raw_content = required_content(args.content, args.content_file.as_deref())?;
    workflow::validate_memory(
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

    println!(
        "{}",
        json!({"status": "superseded", "old_id": old.id, "new_id": new_id})
    );
    Ok(())
}

pub(crate) fn cmd_delete(app: &App, args: DeleteArgs) -> Result<()> {
    app.init()?;
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
        println!(
            "{}",
            json!({"status": "rejected", "reason": "protected_memory_requires_force", "id": old.id})
        );
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
        println!(
            "{}",
            json!({"status": "deleted", "mode": "hard", "id": old.id})
        );
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
        println!(
            "{}",
            json!({"status": "deleted", "mode": "soft", "id": old.id})
        );
    }
    Ok(())
}

pub(crate) fn cmd_context(args: ContextArgs) -> Result<()> {
    if !args.detect {
        bail!("use --detect");
    }
    print_json(&json!({"scope": scope::detect_scope()?}))?;
    Ok(())
}

pub(crate) fn cmd_history(app: &App, args: HistoryArgs) -> Result<()> {
    app.init()?;
    let conn = app.conn()?;
    let mut sql = String::from(
        "SELECT changelog.id, memory_id, action, old_content, new_content, source, changelog.created_at
         FROM changelog",
    );
    let mut clauses = Vec::new();
    let mut bind_values = Vec::new();
    if let Some(name) = args.name {
        let memory_id = memory_by_name(&conn, &name)?
            .map(|m| m.id)
            .ok_or_else(|| anyhow!("memory not found: {name}"))?;
        clauses.push("memory_id = ?");
        bind_values.push(rusqlite::types::Value::Text(memory_id));
    }
    if let Some(action) = args.action {
        clauses.push("action = ?");
        bind_values.push(rusqlite::types::Value::Text(action));
    }
    if !clauses.is_empty() {
        sql.push_str(" WHERE ");
        sql.push_str(&clauses.join(" AND "));
    }
    sql.push_str(" ORDER BY changelog.created_at DESC LIMIT ?");
    bind_values.push(rusqlite::types::Value::Integer(args.limit as i64));

    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(rusqlite::params_from_iter(bind_values), |row| {
        Ok(json!({
            "id": row.get::<_, i64>(0)?,
            "memory_id": row.get::<_, String>(1)?,
            "action": row.get::<_, String>(2)?,
            "old_content": row.get::<_, Option<String>>(3)?,
            "new_content": row.get::<_, Option<String>>(4)?,
            "source": row.get::<_, Option<String>>(5)?,
            "created_at": row.get::<_, String>(6)?,
        }))
    })?;

    let values: Result<Vec<_>, _> = rows.collect();
    print_json_pretty(&values?)?;
    Ok(())
}

pub(crate) fn cmd_stats(app: &App) -> Result<()> {
    app.init()?;
    let conn = app.conn()?;
    print_json_pretty(&stats_report(&conn)?)?;
    Ok(())
}

pub(crate) fn cmd_audit(app: &App, args: AuditArgs) -> Result<()> {
    app.init()?;
    let conn = app.conn()?;
    let report = audit_report(&conn, app, args.fix)?;
    print_json_pretty(&report)?;
    Ok(())
}

fn stats_report(conn: &Connection) -> Result<Value> {
    let total: i64 = conn.query_row(
        "SELECT COUNT(*) FROM memories WHERE valid_until IS NULL",
        [],
        |r| r.get(0),
    )?;
    let by_type = grouped_count(conn, "type")?;
    let by_scope = grouped_count(conn, "scope")?;
    let by_confidence = grouped_count(conn, "confidence")?;
    let top_accessed = query_json_rows(
        conn,
        "SELECT name, access_count, last_accessed_at FROM memories WHERE valid_until IS NULL ORDER BY access_count DESC LIMIT 10",
    )?;
    Ok(json!({
        "total_active": total,
        "by_type": by_type,
        "by_scope": by_scope,
        "by_confidence": by_confidence,
        "top_accessed": top_accessed
    }))
}

fn audit_report(conn: &Connection, app: &App, fix: bool) -> Result<Value> {
    let broken = query_json_rows(
        conn,
        "SELECT name, superseded_by FROM memories
         WHERE superseded_by IS NOT NULL
         AND superseded_by NOT IN (SELECT id FROM memories)",
    )?;
    let expired = query_json_rows(
        conn,
        "SELECT name, expires_at FROM memories
         WHERE expires_at IS NOT NULL AND datetime(expires_at) < datetime('now') AND valid_until IS NULL",
    )?;
    let stale_low_access = query_json_rows(
        conn,
        "SELECT name, created_at, access_count FROM memories
         WHERE access_count = 0 AND datetime(created_at) < datetime('now', '-30 day') AND valid_until IS NULL",
    )?;
    let low_confidence_high_access = query_json_rows(
        conn,
        "SELECT name, confidence, access_count, last_accessed_at FROM memories
         WHERE confidence = 'low' AND access_count >= 3 AND valid_until IS NULL
         ORDER BY access_count DESC",
    )?;
    let cleanup_candidates = query_json_rows(
        conn,
        "SELECT name, confidence, created_at, access_count FROM memories
         WHERE access_count = 0
         AND confidence IN ('low', 'medium')
         AND datetime(created_at) < datetime('now', '-60 day')
         AND valid_until IS NULL
         ORDER BY created_at ASC",
    )?;

    let mut fixed_expired = 0;
    let mut fixed_broken_links = 0;
    if fix {
        let now = now();
        let expired_memories = active_expired_memories(conn)?;
        with_transaction(conn, |conn| {
            for memory in &expired_memories {
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
                    "audit",
                )?;
            }
            fixed_expired = expired_memories.len();
            fixed_broken_links = conn.execute(
                "UPDATE memories
                 SET superseded_by = NULL
                 WHERE superseded_by IS NOT NULL
                 AND superseded_by NOT IN (SELECT id FROM memories)",
                [],
            )?;
            Ok(())
        })?;
        memory_index::reindex_or_mark_stale(app, "rebuild index after audit --fix")?;
    }

    Ok(json!({
        "broken_superseded_links": broken,
        "expired_active_memories": expired,
        "stale_low_access": stale_low_access,
        "low_confidence_high_access": low_confidence_high_access,
        "cleanup_candidates": cleanup_candidates,
        "fixed": fix,
        "fixed_expired": fixed_expired,
        "fixed_broken_links": fixed_broken_links
    }))
}

pub(crate) fn cmd_gc(app: &App, args: GcArgs) -> Result<()> {
    app.init()?;
    let conn = app.conn()?;
    let cutoff = (Utc::now() - Duration::days(args.days)).to_rfc3339();
    let changed = with_transaction(&conn, |conn| {
        let gc_memories = gc_candidate_memories(conn, &cutoff)?;
        for memory in &gc_memories {
            log_change(
                conn,
                &memory.id,
                "gc",
                memory.content.as_deref(),
                None,
                "gc",
            )?;
        }
        let changed = conn.execute(
            "DELETE FROM memories WHERE valid_until IS NOT NULL AND datetime(valid_until) < datetime(?1)",
            params![cutoff],
        )?;
        Ok(changed)
    })?;
    memory_index::reindex_or_mark_stale(app, "rebuild index after gc")?;
    print_json(&json!({"status": "gc_complete", "deleted": changed}))?;
    Ok(())
}

pub(crate) fn cmd_export(app: &App, args: ExportArgs) -> Result<()> {
    app.init()?;
    let conn = app.conn()?;
    let mut memories = all_memories(&conn)?;
    if !args.include_superseded {
        memories.retain(|m| m.valid_until.is_none());
    }

    match args.format {
        ExportFormat::Json => print_json_pretty(&memories)?,
        ExportFormat::Markdown => {
            for memory in memories {
                println!("## {}", memory.name);
                println!();
                println!("- id: {}", memory.id);
                println!("- type: {}", memory.r#type);
                println!("- scope: {}", memory.scope);
                println!("- confidence: {}", memory.confidence);
                println!("- tags: {}", memory.tags);
                println!();
                if let Some(description) = memory.description {
                    println!("{}", description);
                    println!();
                }
                if let Some(content) = memory.content {
                    println!("{}", content);
                    println!();
                }
            }
        }
    }
    Ok(())
}

fn save_args_from_import_value(
    value: Value,
    source: &str,
    no_validate_workflow: bool,
) -> Result<SaveArgs> {
    let name = value
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("import item missing name"))?;
    let content = value
        .get("content")
        .and_then(Value::as_str)
        .unwrap_or_default();
    Ok(SaveArgs {
        r#type: value
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or("reference")
            .to_string(),
        name: name.to_string(),
        description: value
            .get("description")
            .and_then(Value::as_str)
            .map(str::to_string),
        content: Some(content.to_string()),
        content_file: None,
        tags: import_tags(&value)?,
        scope: value
            .get("scope")
            .and_then(Value::as_str)
            .unwrap_or("global")
            .to_string(),
        source: source.to_string(),
        confidence: None,
        expires_at: value
            .get("expires_at")
            .and_then(Value::as_str)
            .map(str::to_string),
        why: None,
        force: false,
        no_validate_workflow,
    })
}

fn result_status(result: &Value) -> String {
    result
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or("unknown")
        .to_string()
}

fn import_tags(value: &Value) -> Result<String> {
    match value.get("tags") {
        Some(Value::String(tags)) => {
            validate_tags(tags)?;
            Ok(tags.clone())
        }
        Some(tags) => {
            let tags = tags.to_string();
            validate_tags(&tags)?;
            Ok(tags)
        }
        None => Ok("[]".to_string()),
    }
}

fn increment_count(counts: &mut serde_json::Map<String, Value>, status: &str) {
    let current = counts.get(status).and_then(Value::as_u64).unwrap_or(0);
    counts.insert(status.to_string(), json!(current + 1));
}

pub(crate) fn cmd_import(app: &App, args: ImportArgs) -> Result<()> {
    app.init()?;
    let text =
        fs::read_to_string(&args.file).with_context(|| format!("read {}", args.file.display()))?;
    let mut results = Vec::new();
    let mut counts = serde_json::Map::new();

    if args.file.extension().and_then(|s| s.to_str()) == Some("json") {
        let values: Vec<Value> = serde_json::from_str(&text).context("parse json import")?;
        for (index, value) in values.into_iter().enumerate() {
            let import_result =
                save_args_from_import_value(value, &args.source, args.no_validate_workflow)
                    .and_then(|save_args| save_memory(app, save_args));
            match import_result {
                Ok(result) => {
                    let status = result_status(&result);
                    increment_count(&mut counts, &status);
                    results.push(json!({
                        "index": index,
                        "status": status,
                        "result": result
                    }));
                }
                Err(err) => {
                    increment_count(&mut counts, "failed");
                    results.push(json!({
                        "index": index,
                        "status": "failed",
                        "error": err.to_string()
                    }));
                }
            }
        }
    } else {
        let name = args
            .file
            .file_stem()
            .and_then(|s| s.to_str())
            .ok_or_else(|| anyhow!("cannot infer name from file"))?
            .to_string();
        let result = save_memory(
            app,
            SaveArgs {
                r#type: args.r#type.unwrap_or_else(|| "reference".to_string()),
                name,
                description: None,
                content: Some(text),
                content_file: None,
                tags: "[]".to_string(),
                scope: "global".to_string(),
                source: args.source,
                confidence: None,
                expires_at: None,
                why: None,
                force: false,
                no_validate_workflow: args.no_validate_workflow,
            },
        )?;
        let status = result_status(&result);
        increment_count(&mut counts, &status);
        results.push(json!({
            "index": 0,
            "status": status,
            "result": result
        }));
    }
    print_json_pretty(&json!({
        "status": "import_complete",
        "total": results.len(),
        "counts": Value::Object(counts),
        "results": results
    }))?;
    Ok(())
}

pub(crate) fn cmd_merge(app: &App, args: MergeArgs) -> Result<()> {
    app.init()?;
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
            if let Err(err) = workflow::validate_record_content(&memory) {
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

pub(crate) fn cmd_retro(app: &App, command: RetroCommand) -> Result<()> {
    app.init()?;
    let conn = app.conn()?;
    let (kind, limit) = match command {
        RetroCommand::Daily(args) => ("daily", args.limit),
        RetroCommand::Weekly(args) => ("weekly", args.limit),
    };
    let limit = limit.clamp(1, 500);
    let recent_history = query_json_rows(
        &conn,
        &format!(
            "SELECT id, memory_id, action, old_content, new_content, source, created_at
             FROM changelog
             ORDER BY created_at DESC
             LIMIT {limit}"
        ),
    )?;
    let pending_ambiguities = ambiguity_rows(&conn, true)?;
    let active_memories = query_json_rows(
        &conn,
        &format!(
            "SELECT id, type, name, tags, scope, source, confidence, version, access_count, updated_at
             FROM memories
             WHERE valid_until IS NULL
             ORDER BY updated_at DESC
             LIMIT {limit}"
        ),
    )?;
    let instructions = match kind {
        "daily" => vec![
            "Use platform-provided conversation context; repo readers are optional adapters.",
            "Compare today's conversation facts against active_memories.",
            "Persist durable new facts with source=daily_retro.",
            "Detect repeated manual procedures and suggest type=workflow memories.",
            "Use update/supersede/delete with --expected-version when changing existing memory.",
            "Record unresolved conflicts with mem ambiguity add.",
        ],
        _ => vec![
            "Review memory quality from changelog, audit, and pending ambiguities.",
            "Merge duplicates, resolve ambiguities, and identify workflow/profile/skill candidates.",
            "Detect stale workflow steps and workflows with repeated failures.",
            "Prefer repeated project procedures as workflow memory; reserve skills for stable cross-project execution policy.",
            "Calibrate low-confidence high-access memories after review.",
            "Use audit --fix only for deterministic repairs.",
        ],
    };

    print_json_pretty(&json!({
        "status": "retro_bundle",
        "kind": kind,
        "generated_at": now(),
        "instructions": instructions,
        "stats": stats_report(&conn)?,
        "audit": audit_report(&conn, app, false)?,
        "pending_ambiguities": pending_ambiguities,
        "recent_history": recent_history,
        "active_memories": active_memories
    }))?;
    Ok(())
}

pub(crate) fn cmd_workflow(app: &App, command: WorkflowCommand) -> Result<()> {
    app.init()?;
    let conn = app.conn()?;
    match command {
        WorkflowCommand::List(args) => {
            let scope_filter = workflow_scope_filter(args.scope.as_deref())?;
            let mut workflows = all_workflows(&conn, args.include_superseded)?;
            workflow::retain_scope(&mut workflows, scope_filter.as_deref());
            workflows.truncate(args.limit);
            print_json_pretty(&workflows)?;
        }
        WorkflowCommand::Show(args) => {
            let workflow = workflow_by_ref(&conn, &args.reference)?;
            print_json_pretty(&workflow)?;
        }
        WorkflowCommand::Find(args) => {
            let scope_filter = workflow_scope_filter(args.scope.as_deref())?;
            let mut workflows = all_workflows(&conn, false)?;
            workflow::retain_scope(&mut workflows, scope_filter.as_deref());
            workflows.retain(|memory| workflow::matches_intent(memory, &args.intent));
            workflows.sort_by_key(|workflow| {
                std::cmp::Reverse(workflow::rank(
                    workflow,
                    &args.intent,
                    scope_filter.as_deref(),
                ))
            });
            workflows.truncate(args.limit);
            print_json_pretty(&workflows)?;
        }
        WorkflowCommand::Validate(args) => {
            let workflow = workflow_by_ref(&conn, &args.reference)?;
            workflow::validate_record(&workflow)?;
            print_json_pretty(&json!({
                "status": "valid",
                "id": workflow.id,
                "name": workflow.name
            }))?;
        }
    }
    Ok(())
}

pub(crate) fn cmd_ambiguity(app: &App, command: AmbiguityCommand) -> Result<()> {
    app.init()?;
    let conn = app.conn()?;
    match command {
        AmbiguityCommand::Add(args) => {
            validate_tags(&args.memory_ids)?;
            let memory_ids = parse_string_array(&args.memory_ids)?;
            add_ambiguity_record(&conn, &args.query, &memory_ids, args.context.as_deref())?;
            println!(
                "{}",
                json!({"status": "ambiguity_added", "id": conn.last_insert_rowid()})
            );
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
            if args.soft_delete_others {
                let keep_id = keep_id
                    .as_deref()
                    .ok_or_else(|| anyhow!("--soft-delete-others requires --keep"))?;
                for memory_id in memory_ids.iter().filter(|id| id.as_str() != keep_id) {
                    let Some(memory) = memory_by_id(&conn, memory_id)? else {
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
                        &conn,
                        &memory.id,
                        "delete",
                        memory.content.as_deref(),
                        None,
                        "ambiguity_resolve",
                    )?;
                    soft_deleted.push(memory.id);
                }
                if !soft_deleted.is_empty() {
                    memory_index::reindex_or_mark_stale(
                        app,
                        "rebuild index after ambiguity resolution",
                    )?;
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
            print_json_pretty(&json!({
                "status": "resolved",
                "id": args.id,
                "resolution": serde_json::from_str::<Value>(&resolution)?
            }))?;
        }
    }
    Ok(())
}

fn workflow_scope_filter(scope: Option<&str>) -> Result<Option<Vec<String>>> {
    match scope {
        Some("auto") => Ok(Some(scope::detect_scope_set()?)),
        Some(scope) => Ok(Some(vec!["global".to_string(), scope.to_string()])),
        None => Ok(None),
    }
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

fn similar_candidates(
    app: &App,
    conn: &Connection,
    content: &str,
    limit: usize,
) -> Result<Vec<Value>> {
    let ids = memory_index::search_ids(app, content, false, false, 25)?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_app(name: &str) -> App {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("agent-knowledge-main-{name}-{stamp}"));
        fs::create_dir_all(root.join("schema")).expect("schema dir");
        fs::write(
            root.join("schema/memory-schema.sql"),
            include_str!("../../../schema/memory-schema.sql"),
        )
        .expect("schema");
        App {
            db_path: root.join("memory.db"),
            index_path: root.join("index"),
            schema_path: root.join("schema/memory-schema.sql"),
            root,
        }
    }

    #[test]
    fn upsert_index_failure_marks_stale() {
        let app = temp_app("upsert-stale");
        app.init().expect("init app");
        let conn = app.conn().expect("open db");
        conn.execute(
            "INSERT INTO memories
            (id, type, name, content, tags, scope, source, confidence, protected, created_at, updated_at)
            VALUES ('broken_index', 'reference', 'broken_index', 'content', '[]', 'global', 'manual', 'high', 1, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
            [],
        )
        .expect("insert memory");

        fs::remove_dir_all(&app.index_path).expect("remove index");
        fs::create_dir_all(&app.index_path).expect("index dir");
        fs::write(
            app.index_path.join("meta.json"),
            "not valid tantivy metadata",
        )
        .expect("corrupt index");

        let result = memory_index::upsert_or_mark_stale(&app, &conn, "broken_index");

        assert!(result.is_err());
        assert!(memory_index::is_stale(&app));
        assert!(memory_index::dirty_in_db(&app).expect("index dirty state"));
        fs::remove_dir_all(app.root).ok();
    }
}
