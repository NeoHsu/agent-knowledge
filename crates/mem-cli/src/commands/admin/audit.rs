use super::super::*;

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
