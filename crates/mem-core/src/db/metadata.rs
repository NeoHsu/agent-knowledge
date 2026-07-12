use super::*;

pub const STORE_ID_KEY: &str = "store_id";

pub fn ensure_store_id(conn: &Connection) -> Result<String> {
    if let Some(id) = conn
        .query_row(
            "SELECT value FROM metadata WHERE key = ?1",
            params![STORE_ID_KEY],
            |row| row.get::<_, String>(0),
        )
        .optional()?
    {
        return Ok(id);
    }
    let id = uuid::Uuid::new_v4().to_string();
    conn.execute(
        "INSERT INTO metadata (key, value, updated_at) VALUES (?1, ?2, ?3)",
        params![STORE_ID_KEY, id, now()],
    )?;
    Ok(id)
}

pub fn store_id(conn: &Connection) -> Result<String> {
    conn.query_row(
        "SELECT value FROM metadata WHERE key = ?1",
        params![STORE_ID_KEY],
        |row| row.get(0),
    )
    .context("store_id metadata is missing; run `mem migrate`")
}

pub fn new_event_uid(conn: &Connection, kind: &str) -> Result<String> {
    Ok(format!(
        "{}:{kind}:{}",
        store_id(conn)?,
        uuid::Uuid::new_v4()
    ))
}

pub fn set_index_dirty(conn: &Connection, dirty: bool) -> Result<()> {
    conn.execute(
        "INSERT INTO metadata (key, value, updated_at)
         VALUES (?1, ?2, ?3)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at
         WHERE metadata.value <> excluded.value",
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
