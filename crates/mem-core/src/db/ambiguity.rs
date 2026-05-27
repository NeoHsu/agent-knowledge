use super::reporting::parse_json_string_field;
use super::*;

pub fn add_ambiguity_record(
    conn: &Connection,
    query: &str,
    memory_ids: &[String],
    context: Option<&str>,
) -> Result<()> {
    let memory_ids = serde_json::to_string(memory_ids)?;
    conn.execute(
        "INSERT INTO ambiguities (query, memory_ids, context, resolution)
         VALUES (?1, ?2, ?3, 'pending')",
        params![query, memory_ids, context],
    )?;
    Ok(())
}

pub fn ambiguity_rows(conn: &Connection, pending_only: bool) -> Result<Vec<Value>> {
    let sql = if pending_only {
        "SELECT id, query, memory_ids, context, resolution, created_at, resolved_at
         FROM ambiguities
         WHERE resolution = 'pending'
         ORDER BY created_at DESC"
    } else {
        "SELECT id, query, memory_ids, context, resolution, created_at, resolved_at
         FROM ambiguities
         ORDER BY created_at DESC"
    };
    let mut rows = query_json_rows(conn, sql)?;
    for row in &mut rows {
        parse_json_string_field(row, "memory_ids");
        parse_json_string_field(row, "context");
        parse_json_string_field(row, "resolution");
    }
    Ok(rows)
}

pub fn ambiguity_by_id(conn: &Connection, id: i64) -> Result<Option<Value>> {
    conn.query_row(
        "SELECT id, query, memory_ids, context, resolution, created_at, resolved_at
         FROM ambiguities
         WHERE id = ?1",
        params![id],
        |row| {
            Ok(json!({
                "id": row.get::<_, i64>(0)?,
                "query": row.get::<_, String>(1)?,
                "memory_ids": row.get::<_, String>(2)?,
                "context": row.get::<_, Option<String>>(3)?,
                "resolution": row.get::<_, Option<String>>(4)?,
                "created_at": row.get::<_, String>(5)?,
                "resolved_at": row.get::<_, Option<String>>(6)?,
            }))
        },
    )
    .optional()
    .map_err(Into::into)
}
