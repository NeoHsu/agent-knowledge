use super::super::*;
use super::render::{render_history_compact, render_history_table};

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
