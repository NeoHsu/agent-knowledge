use super::*;

/// Record one execution of a workflow runbook. Runs are local telemetry for
/// retro quality loops; they are not memories and are never indexed.
pub fn log_workflow_run(
    conn: &Connection,
    memory_id: &str,
    result: &str,
    note: Option<&str>,
    source: &str,
) -> Result<i64> {
    let uid = new_event_uid(conn, "workflow-run")?;
    conn.execute(
        "INSERT INTO workflow_runs (uid, memory_id, result, note, source, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        rusqlite::params![uid, memory_id, result, note, source, crate::util::now()],
    )
    .context("insert workflow run")?;
    Ok(conn.last_insert_rowid())
}

/// (total runs, failures) for one workflow memory.
pub fn workflow_run_counts(conn: &Connection, memory_id: &str) -> Result<(i64, i64)> {
    conn.query_row(
        "SELECT COUNT(*), COALESCE(SUM(result = 'failure'), 0)
         FROM workflow_runs WHERE memory_id = ?1",
        [memory_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )
    .context("count workflow runs")
}

/// Per-workflow run statistics for retro bundles, most recently run first.
pub fn workflow_run_stats(conn: &Connection, limit: usize) -> Result<Vec<serde_json::Value>> {
    query_json_rows(
        conn,
        &format!(
            "SELECT r.memory_id, m.name,
                    COUNT(*) AS runs,
                    COALESCE(SUM(r.result = 'failure'), 0) AS failures,
                    MAX(r.created_at) AS last_run_at,
                    (SELECT r2.result FROM workflow_runs r2
                     WHERE r2.memory_id = r.memory_id
                     ORDER BY r2.created_at DESC, r2.id DESC LIMIT 1) AS last_result
             FROM workflow_runs r
             LEFT JOIN memories m ON m.id = r.memory_id
             GROUP BY r.memory_id
             ORDER BY last_run_at DESC
             LIMIT {limit}"
        ),
    )
}
