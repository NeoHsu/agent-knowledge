use super::*;

pub fn log_change(
    conn: &Connection,
    memory_id: &str,
    action: &str,
    old_content: Option<&str>,
    new_content: Option<&str>,
    source: &str,
) -> Result<()> {
    let uid = new_event_uid(conn, "change")?;
    conn.execute(
        "INSERT INTO changelog (uid, memory_id, action, old_content, new_content, source)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![uid, memory_id, action, old_content, new_content, source],
    )?;
    Ok(())
}
