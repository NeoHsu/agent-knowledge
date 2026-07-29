use super::super::*;
use super::render::{render_stats_compact, render_stats_table};

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
