use super::*;

pub fn set_index_dirty(conn: &Connection, dirty: bool) -> Result<()> {
    conn.execute(
        "INSERT INTO metadata (key, value, updated_at)
         VALUES (?1, ?2, ?3)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
        params![INDEX_DIRTY_KEY, if dirty { "true" } else { "false" }, now()],
    )?;
    Ok(())
}

pub fn index_dirty(conn: &Connection) -> Result<bool> {
    let value: Option<String> = conn
        .query_row(
            "SELECT value FROM metadata WHERE key = ?1",
            params![INDEX_DIRTY_KEY],
            |row| row.get(0),
        )
        .optional()?;
    Ok(value.as_deref() == Some("true"))
}
