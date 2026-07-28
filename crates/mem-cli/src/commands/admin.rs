use super::*;
use mem_core::config::user_config_path;

pub(crate) fn cmd_migrate(app: &App, args: MigrateArgs) -> Result<()> {
    let current = app.schema_version()?;
    let target = mem_core::db::supported_schema_version();
    if current > target {
        bail!("database schema v{current} is newer than this binary supports (v{target})");
    }
    if args.dry_run {
        let compatibility_required = if current == target {
            let conn = app.read_conn()?;
            let required = mem_core::db::schema_compatibility_required(&conn)?;
            if !required {
                mem_core::db::validate_store_schema_objects(&conn).context(
                    "store contains unexpected schema objects; migration cannot repair untrusted DDL",
                )?;
            }
            required
        } else {
            false
        };
        let migration_required = current < target || compatibility_required;
        return print_json_pretty(&json!({
            "status": "dry_run",
            "root": app.root.display().to_string(),
            "current_schema": current,
            "target_schema": target,
            "migration_required": migration_required,
            "compatibility_repair_required": compatibility_required,
            "backup_required": migration_required,
        }));
    }
    let backup = app.migrate()?;
    print_json_pretty(&json!({
        "status": if backup.is_some() { "migrated" } else { "up_to_date" },
        "root": app.root.display().to_string(),
        "from_schema": current,
        "to_schema": target,
        "backup": backup.map(|path| path.display().to_string()),
    }))
}

pub(crate) fn cmd_context(args: ContextArgs) -> Result<()> {
    if !args.detect {
        bail!("missing required action. Try `mem context --detect` to show the detected project scope, or `mem context --help` for options.");
    }
    print_json(&json!({"scope": scope::detect_scope()?}))?;
    Ok(())
}

pub(crate) fn cmd_contract() -> Result<()> {
    print_json_pretty(&json!({
        "status": "ok",
        "contract_version": CLI_OUTPUT_CONTRACT_VERSION,
        "cli_version": env!("CARGO_PKG_VERSION"),
        "compatibility": {
            "successful_json": "required fields remain compatible within a minor release; additive fields are allowed",
            "json_errors": "versioned by contract_version and emitted only with --json-errors",
            "pre_1_0_breaking_changes": "may occur only in a documented minor release"
        },
        "json_errors": {
            "version": CLI_OUTPUT_CONTRACT_VERSION,
            "required_fields": ["status", "contract_version", "code", "message", "exit_code"],
            "optional_fields": ["details"],
            "known_codes": ["cli_parse_error", "command_failed", "index_stale_after_write"]
        },
        "schemas": {
            "store": supported_schema_version(),
            "bundle": super::bundle::BUNDLE_FORMAT_VERSION,
            "workflow": mem_core::workflow::WORKFLOW_SCHEMA_VERSION,
            "graph": mem_core::graph::GRAPH_SCHEMA_VERSION,
            "benchmark_report": BENCHMARK_REPORT_CONTRACT_VERSION
        }
    }))
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
                    "MNEMARK_HOME": std::env::var("MNEMARK_HOME").ok(),
                    "XDG_CONFIG_HOME": std::env::var("XDG_CONFIG_HOME").ok()
                },
                "effective": {
                    "knowledge_home": app.root.display().to_string(),
                    "schema": "embedded",
                    "query_default_scope": app.config.query_default_scope(),
                    "query_default_limit": app.config.query_default_limit().unwrap_or(DEFAULT_LIMIT),
                    "query_candidate_limit": app.config.query_candidate_limit(),
                    "workflow_default_scope": app.config.workflow_default_scope(),
                    "workflow_default_limit": app.config.workflow_default_limit().unwrap_or(DEFAULT_LIMIT),
                    "budget_per_scope_max": app.config.per_scope_max()
                },
                "config": app.config
            }))?;
        }
    }
    Ok(())
}

pub(crate) fn cmd_history(app: &App, args: HistoryArgs) -> Result<()> {
    app.require_schema()?;
    let conn = app.read_conn()?;
    let mut sql = String::from(
        "SELECT changelog.id, memory_id, action, old_content, new_content, source, changelog.created_at
         FROM changelog",
    );
    let mut clauses = Vec::new();
    let mut bind_values = Vec::new();
    if let Some(name) = args.name {
        let scopes = if args.scope == "auto" {
            scope::detect_scope_set()?
        } else {
            scope::validate_scope(&args.scope)?;
            vec![args.scope.clone()]
        };
        let scope_refs = scopes.iter().map(String::as_str).collect::<Vec<_>>();
        let memory_id = resolve_memory_ref_in_scopes(&conn, &name, Some(&scope_refs))?;
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
    let values = values?;
    match args.format {
        OutputFormat::Json => print_json_pretty(&values)?,
        OutputFormat::Table => print_text(render_history_table(&values))?,
        OutputFormat::Compact => print_text(render_history_compact(&values))?,
    }
    Ok(())
}

pub(crate) fn cmd_stats(app: &App, args: StatsArgs) -> Result<()> {
    app.require_schema()?;
    let conn = app.read_conn()?;
    let report = stats_report(&conn)?;
    match args.format {
        OutputFormat::Json => print_json_pretty(&report)?,
        OutputFormat::Table => print_text(render_stats_table(&report))?,
        OutputFormat::Compact => print_text(render_stats_compact(&report))?,
    }
    Ok(())
}

pub(crate) fn cmd_audit(app: &App, args: AuditArgs) -> Result<()> {
    app.require_schema()?;
    let conn = if args.fix {
        app.conn()?
    } else {
        app.read_conn()?
    };
    let report = audit_report(&conn, app, args.fix)?;
    print_json_pretty(&report)?;
    Ok(())
}

pub(crate) fn stats_report(conn: &Connection) -> Result<Value> {
    let total_sql = format!("SELECT COUNT(*) FROM memories WHERE {ACTIVE_MEMORY_SQL}");
    let total: i64 = conn.query_row(&total_sql, [], |r| r.get(0))?;
    let by_type = grouped_count(conn, "type")?;
    let by_scope = grouped_count(conn, "scope")?;
    let by_confidence = grouped_count(conn, "confidence")?;
    let top_accessed = query_json_rows(
        conn,
        &format!(
            "SELECT name, access_count, last_accessed_at FROM memories \
             WHERE {ACTIVE_MEMORY_SQL} ORDER BY access_count DESC LIMIT 10"
        ),
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
         WHERE access_count = 0 AND datetime(created_at) < datetime('now', '-30 day')
           AND valid_until IS NULL
           AND (expires_at IS NULL OR datetime(expires_at) >= datetime('now'))",
    )?;
    let low_confidence_high_access = query_json_rows(
        conn,
        "SELECT name, confidence, access_count, last_accessed_at FROM memories
         WHERE confidence = 'low' AND access_count >= 3 AND valid_until IS NULL
           AND (expires_at IS NULL OR datetime(expires_at) >= datetime('now'))
         ORDER BY access_count DESC",
    )?;
    let cleanup_candidates = query_json_rows(
        conn,
        "SELECT name, confidence, created_at, access_count FROM memories
         WHERE access_count = 0
         AND confidence IN ('low', 'medium')
         AND datetime(created_at) < datetime('now', '-60 day')
         AND valid_until IS NULL
         AND (expires_at IS NULL OR datetime(expires_at) >= datetime('now'))
         ORDER BY created_at ASC",
    )?;

    let per_scope_max = app.config.per_scope_max();
    let over_budget_scopes = if per_scope_max == 0 {
        Vec::new()
    } else {
        over_budget_scope_rows(conn, per_scope_max)?
    };
    let graph = mem_core::graph::graph_health(conn)?;

    let mut fixed_expired = 0;
    let mut fixed_broken_links = 0;
    if fix {
        let now = now();
        let expired_memories = active_expired_memories(conn)?;
        with_transaction(conn, |conn| {
            for memory in &expired_memories {
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
                    "audit",
                )?;
            }
            fixed_expired = expired_memories.len();
            fixed_broken_links = conn.execute(
                "UPDATE memories
                 SET superseded_by = NULL, updated_at = ?1, version = version + 1
                 WHERE superseded_by IS NOT NULL
                 AND superseded_by NOT IN (SELECT id FROM memories)",
                params![now],
            )?;
            Ok(())
        })?;
        mem_core::graph::set_graph_dirty(conn, true)?;
        finish_committed_index_write(
            memory_index::reindex_or_mark_stale(app, "rebuild index after audit --fix"),
            "audit repair",
            json!({
                "fixed_expired": fixed_expired,
                "fixed_broken_links": fixed_broken_links
            }),
        )?;
    }

    Ok(json!({
        "broken_superseded_links": broken,
        "expired_active_memories": expired,
        "stale_low_access": stale_low_access,
        "low_confidence_high_access": low_confidence_high_access,
        "cleanup_candidates": cleanup_candidates,
        "per_scope_max": per_scope_max,
        "over_budget_scopes": over_budget_scopes,
        "graph": graph,
        "fixed": fix,
        "fixed_expired": fixed_expired,
        "fixed_broken_links": fixed_broken_links
    }))
}

/// Scopes holding more active memories than the soft budget, each with its
/// lowest-value curation candidates (protected manual memories excluded).
/// The cap never blocks writes; it exists to force curation at audit/retro
/// time instead of letting scopes grow without bound.
fn over_budget_scope_rows(conn: &Connection, per_scope_max: usize) -> Result<Vec<Value>> {
    let scopes = query_json_rows(
        conn,
        &format!(
            "SELECT scope, COUNT(*) AS count FROM memories
             WHERE valid_until IS NULL
               AND (expires_at IS NULL OR datetime(expires_at) >= datetime('now'))
             GROUP BY scope
             HAVING COUNT(*) > {per_scope_max}
             ORDER BY count DESC"
        ),
    )?;
    let mut rows = Vec::new();
    for entry in scopes {
        let scope = entry["scope"].as_str().unwrap_or_default().to_string();
        let mut stmt = conn.prepare(
            "SELECT name, confidence, access_count, created_at FROM memories
             WHERE valid_until IS NULL AND scope = ?1 AND protected = 0
               AND (expires_at IS NULL OR datetime(expires_at) >= datetime('now'))
             ORDER BY access_count ASC, created_at ASC
             LIMIT 10",
        )?;
        let candidates = stmt
            .query_map(params![scope], |row| {
                Ok(json!({
                    "name": row.get::<_, String>(0)?,
                    "confidence": row.get::<_, String>(1)?,
                    "access_count": row.get::<_, i64>(2)?,
                    "created_at": row.get::<_, String>(3)?,
                }))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows.push(json!({
            "scope": scope,
            "count": entry["count"],
            "per_scope_max": per_scope_max,
            "curation_candidates": candidates
        }));
    }
    Ok(rows)
}

pub(crate) fn cmd_gc(app: &App, args: GcArgs) -> Result<()> {
    app.require_schema()?;
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
    mem_core::graph::set_graph_dirty(&conn, true)?;
    finish_committed_index_write(
        memory_index::reindex_or_mark_stale(app, "rebuild index after gc"),
        "garbage collection",
        json!({"deleted": changed}),
    )?;
    print_json(&json!({"status": "gc_complete", "deleted": changed}))?;
    Ok(())
}

fn render_history_table(rows: &[Value]) -> String {
    let table_rows = rows
        .iter()
        .map(|row| {
            vec![
                value_text(row, "id"),
                value_text(row, "action"),
                truncate_text(&value_text(row, "memory_id"), 32),
                value_text(row, "source"),
                truncate_text(&value_text(row, "created_at"), 20),
            ]
        })
        .collect::<Vec<_>>();
    render_table(
        &["id", "action", "memory_id", "source", "created"],
        &table_rows,
    )
}

fn render_history_compact(rows: &[Value]) -> String {
    let mut output = String::new();
    for row in rows {
        output.push_str(&format!(
            "{} {} {} source={}",
            value_text(row, "created_at"),
            value_text(row, "action"),
            value_text(row, "memory_id"),
            value_text(row, "source")
        ));
        output.push('\n');
        let old_content = value_text(row, "old_content");
        if !old_content.is_empty() {
            output.push_str(&format!("  old: {}\n", truncate_text(&old_content, 120)));
        }
        let new_content = value_text(row, "new_content");
        if !new_content.is_empty() {
            output.push_str(&format!("  new: {}\n", truncate_text(&new_content, 120)));
        }
    }
    output
}

fn render_stats_table(report: &Value) -> String {
    let mut rows = vec![vec![
        "total_active".to_string(),
        value_text(report, "total_active"),
    ]];
    append_count_rows(&mut rows, report, "by_type", "type");
    append_count_rows(&mut rows, report, "by_scope", "scope");
    append_count_rows(&mut rows, report, "by_confidence", "confidence");
    let mut output = render_table(&["metric", "value"], &rows);

    let top_rows = report
        .get("top_accessed")
        .and_then(Value::as_array)
        .map(|rows| {
            rows.iter()
                .map(|row| {
                    vec![
                        truncate_text(&value_text(row, "name"), 32),
                        value_text(row, "access_count"),
                        value_text(row, "last_accessed_at"),
                    ]
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if !top_rows.is_empty() {
        output.push('\n');
        output.push_str("top_accessed\n");
        output.push_str(&render_table(
            &["name", "access", "last_accessed"],
            &top_rows,
        ));
    }
    output
}

fn render_stats_compact(report: &Value) -> String {
    let mut output = String::new();
    output.push_str(&format!(
        "total_active: {}\n",
        value_text(report, "total_active")
    ));
    output.push_str(&format!(
        "by_type: {}",
        count_map_text(report.get("by_type")).unwrap_or_else(|| "-".to_string())
    ));
    output.push('\n');
    output.push_str(&format!(
        "by_scope: {}",
        count_map_text(report.get("by_scope")).unwrap_or_else(|| "-".to_string())
    ));
    output.push('\n');
    output.push_str(&format!(
        "by_confidence: {}",
        count_map_text(report.get("by_confidence")).unwrap_or_else(|| "-".to_string())
    ));
    output.push('\n');
    if let Some(rows) = report.get("top_accessed").and_then(Value::as_array) {
        if !rows.is_empty() {
            output.push_str("top_accessed:\n");
            for row in rows {
                output.push_str(&format!(
                    "  {} access={} last={}",
                    value_text(row, "name"),
                    value_text(row, "access_count"),
                    value_text(row, "last_accessed_at")
                ));
                output.push('\n');
            }
        }
    }
    output
}

fn append_count_rows(rows: &mut Vec<Vec<String>>, report: &Value, key: &str, prefix: &str) {
    let Some(map) = report.get(key).and_then(Value::as_object) else {
        return;
    };
    let mut entries = map.iter().collect::<Vec<_>>();
    entries.sort_by(|a, b| a.0.cmp(b.0));
    for (name, value) in entries {
        rows.push(vec![format!("{prefix}:{name}"), scalar_text(value)]);
    }
}

fn count_map_text(value: Option<&Value>) -> Option<String> {
    let map = value?.as_object()?;
    let mut entries = map
        .iter()
        .map(|(key, value)| format!("{key}={}", scalar_text(value)))
        .collect::<Vec<_>>();
    entries.sort();
    Some(entries.join(", "))
}

fn value_text(row: &Value, key: &str) -> String {
    row.get(key).map(scalar_text).unwrap_or_default()
}

fn scalar_text(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::String(value) => value.clone(),
        Value::Number(value) => value.to_string(),
        Value::Bool(value) => value.to_string(),
        other => serde_json::to_string(other).unwrap_or_default(),
    }
}
