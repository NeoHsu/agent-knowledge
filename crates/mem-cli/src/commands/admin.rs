use super::*;
use mem_core::config::user_config_path;

pub(crate) fn cmd_context(args: ContextArgs) -> Result<()> {
    if !args.detect {
        bail!("use --detect");
    }
    print_json(&json!({"scope": scope::detect_scope()?}))?;
    Ok(())
}

pub(crate) fn cmd_config(app: &App, command: ConfigCommand) -> Result<()> {
    match command {
        ConfigCommand::Show => {
            print_json_pretty(&json!({
                "root": app.root.display().to_string(),
                "store_source": app.store_source.as_str(),
                "db_path": app.db_path.display().to_string(),
                "index_path": app.index_path.display().to_string(),
                "user_config_path": user_config_path().display().to_string(),
                "user_config_exists": user_config_path().exists(),
                "store_config_path": app.root.join("config.toml").display().to_string(),
                "store_config_exists": app.root.join("config.toml").exists(),
                "env": {
                    "AGENT_KNOWLEDGE_HOME": std::env::var("AGENT_KNOWLEDGE_HOME").ok(),
                    "XDG_CONFIG_HOME": std::env::var("XDG_CONFIG_HOME").ok()
                },
                "effective": {
                    "knowledge_home": app.root.display().to_string(),
                    "schema": "embedded",
                    "query_default_scope": app.config.query_default_scope(),
                    "query_default_limit": app.config.query_default_limit().unwrap_or(DEFAULT_LIMIT),
                    "workflow_default_scope": app.config.workflow_default_scope(),
                    "workflow_default_limit": app.config.workflow_default_limit().unwrap_or(DEFAULT_LIMIT)
                },
                "config": app.config
            }))?;
        }
    }
    Ok(())
}

pub(crate) fn cmd_history(app: &App, args: HistoryArgs) -> Result<()> {
    app.ensure_schema()?;
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
    app.ensure_schema()?;
    let conn = app.conn()?;
    print_json_pretty(&stats_report(&conn)?)?;
    Ok(())
}

pub(crate) fn cmd_audit(app: &App, args: AuditArgs) -> Result<()> {
    app.ensure_schema()?;
    let conn = app.conn()?;
    let report = audit_report(&conn, app, args.fix)?;
    print_json_pretty(&report)?;
    Ok(())
}

pub(crate) fn stats_report(conn: &Connection) -> Result<Value> {
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

pub(crate) fn audit_report(conn: &Connection, app: &App, fix: bool) -> Result<Value> {
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
    app.ensure_schema()?;
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
