use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};

use rusqlite::OptionalExtension;

use super::report::DurableEventMergeReport;
use super::sanitize::sanitize_optional;
use super::*;

pub(super) fn merge_workflow_runs(
    conn: &Connection,
    incoming: &Connection,
    incoming_store: &str,
    memory_id_map: &HashMap<String, String>,
    allow_secret_redaction: bool,
    report: &mut DurableEventMergeReport,
) -> Result<()> {
    if !mem_core::db::table_exists(incoming, "workflow_runs")? {
        return Ok(());
    }
    let uid_expr = compatible_uid_expression(incoming, "workflow_runs")?;
    let sql = format!(
        "SELECT id, {uid_expr}, memory_id, result, note, source, created_at
         FROM workflow_runs ORDER BY id"
    );
    let mut stmt = incoming.prepare(&sql)?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, Option<String>>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, Option<String>>(4)?,
            row.get::<_, Option<String>>(5)?,
            row.get::<_, String>(6)?,
        ))
    })?;
    for row in rows {
        let (id, uid, memory_id, result, note, source, created_at) = row?;
        let uid = effective_event_uid(uid, incoming_store, "workflow-run", id);
        if local_event_id(conn, "workflow_runs", &uid)?.is_some() {
            report.workflow_runs_identical += 1;
            continue;
        }
        let Some(memory_id) = memory_id_map.get(&memory_id) else {
            report.workflow_runs_unresolved += 1;
            continue;
        };
        let note = note
            .as_deref()
            .map(|value| sanitize_secret_field(value, "workflow run note", allow_secret_redaction))
            .transpose()?;
        if note.as_deref().is_some_and(|value| value.len() > 65_536) {
            bail!("incoming workflow run note exceeds 65536 bytes");
        }
        conn.execute(
            "INSERT INTO workflow_runs
             (uid, memory_id, result, note, source, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![uid, memory_id, result, note, source, created_at],
        )?;
        report.workflow_runs_imported += 1;
    }
    Ok(())
}

pub(super) fn merge_changelog(
    conn: &Connection,
    incoming: &Connection,
    incoming_store: &str,
    memory_id_map: &HashMap<String, String>,
    allow_secret_redaction: bool,
    report: &mut DurableEventMergeReport,
) -> Result<()> {
    if !mem_core::db::table_exists(incoming, "changelog")? {
        return Ok(());
    }
    let uid_expr = compatible_uid_expression(incoming, "changelog")?;
    let sql = format!(
        "SELECT id, {uid_expr}, memory_id, action, old_content, new_content, source, created_at
         FROM changelog ORDER BY id"
    );
    let mut stmt = incoming.prepare(&sql)?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, Option<String>>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, Option<String>>(4)?,
            row.get::<_, Option<String>>(5)?,
            row.get::<_, Option<String>>(6)?,
            row.get::<_, String>(7)?,
        ))
    })?;
    for row in rows {
        let (id, uid, memory_id, action, old_content, new_content, source, created_at) = row?;
        let uid = effective_event_uid(uid, incoming_store, "change", id);
        if local_event_id(conn, "changelog", &uid)?.is_some() {
            report.changelog_identical += 1;
            continue;
        }
        let mapped_memory_id = memory_id_map.get(&memory_id).cloned().unwrap_or_else(|| {
            report.changelog_unresolved += 1;
            namespaced_unmapped_id(incoming_store, &memory_id)
        });
        let old_content = sanitize_optional(
            old_content.as_deref(),
            "changelog old_content",
            allow_secret_redaction,
        )?;
        let new_content = sanitize_optional(
            new_content.as_deref(),
            "changelog new_content",
            allow_secret_redaction,
        )?;
        if old_content
            .as_deref()
            .is_some_and(|value| value.len() > 1_048_576)
            || new_content
                .as_deref()
                .is_some_and(|value| value.len() > 1_048_576)
        {
            bail!("incoming changelog content exceeds 1048576 bytes");
        }
        conn.execute(
            "INSERT INTO changelog
             (uid, memory_id, action, old_content, new_content, source, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                uid,
                mapped_memory_id,
                action,
                old_content,
                new_content,
                source,
                created_at
            ],
        )?;
        report.changelog_imported += 1;
    }
    Ok(())
}

pub(super) fn incoming_store_key(conn: &Connection) -> Result<String> {
    if mem_core::db::table_exists(conn, "metadata")? {
        if let Some(id) = conn
            .query_row(
                "SELECT value FROM metadata WHERE key = 'store_id'",
                [],
                |row| row.get::<_, String>(0),
            )
            .optional()?
        {
            return Ok(id);
        }
    }
    let identity: String = conn.query_row(
        "SELECT COALESCE(group_concat(id || ':' || name, '|'), '')
         FROM (SELECT id, name FROM memories ORDER BY id)",
        [],
        |row| row.get(0),
    )?;
    let mut hasher = DefaultHasher::new();
    identity.hash(&mut hasher);
    Ok(format!("legacy-{:016x}", hasher.finish()))
}

pub(super) fn compatible_uid_expression(conn: &Connection, table: &str) -> Result<&'static str> {
    if mem_core::db::column_exists(conn, table, "uid")? {
        Ok("uid")
    } else {
        Ok("NULL AS uid")
    }
}

pub(super) fn effective_event_uid(
    uid: Option<String>,
    incoming_store: &str,
    kind: &str,
    id: i64,
) -> String {
    uid.filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| format!("{incoming_store}:{kind}:legacy:{id}"))
}

pub(super) fn local_event_id(conn: &Connection, table: &str, uid: &str) -> Result<Option<i64>> {
    conn.query_row(
        &format!("SELECT id FROM {table} WHERE uid = ?1"),
        params![uid],
        |row| row.get(0),
    )
    .optional()
    .map_err(Into::into)
}

pub(super) fn namespaced_unmapped_id(incoming_store: &str, id: &str) -> String {
    format!("unmapped:{incoming_store}:{id}")
}
