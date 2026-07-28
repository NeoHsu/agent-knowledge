use std::collections::HashMap;

use super::events::{
    compatible_uid_expression, effective_event_uid, local_event_id, namespaced_unmapped_id,
};
use super::report::DurableEventMergeReport;
use super::*;

pub(super) fn merge_ambiguities(
    conn: &Connection,
    incoming: &Connection,
    incoming_store: &str,
    memory_id_map: &HashMap<String, String>,
    allow_secret_redaction: bool,
) -> Result<(HashMap<i64, i64>, DurableEventMergeReport)> {
    let mut report = DurableEventMergeReport::default();
    let mut id_map = HashMap::new();
    if !mem_core::db::table_exists(incoming, "ambiguities")? {
        return Ok((id_map, report));
    }
    let uid_expr = compatible_uid_expression(incoming, "ambiguities")?;
    let sql = format!(
        "SELECT id, {uid_expr}, query, memory_ids, context, resolution, created_at, resolved_at
         FROM ambiguities ORDER BY id"
    );
    let mut stmt = incoming.prepare(&sql)?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, Option<String>>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, Option<String>>(4)?,
            row.get::<_, String>(5)?,
            row.get::<_, String>(6)?,
            row.get::<_, Option<String>>(7)?,
        ))
    })?;
    for row in rows {
        let (incoming_id, uid, query, memory_ids, context, resolution, created_at, resolved_at) =
            row?;
        let uid = effective_event_uid(uid, incoming_store, "ambiguity", incoming_id);
        if let Some(local_id) = local_event_id(conn, "ambiguities", &uid)? {
            id_map.insert(incoming_id, local_id);
            report.ambiguities_identical += 1;
            continue;
        }
        let query = sanitize_secret_field(&query, "ambiguity query", allow_secret_redaction)?;
        let context = context
            .as_deref()
            .map(|value| sanitize_secret_field(value, "ambiguity context", allow_secret_redaction))
            .transpose()?;
        let resolution =
            sanitize_secret_field(&resolution, "ambiguity resolution", allow_secret_redaction)?;
        if query.len() > 10_000
            || context
                .as_deref()
                .is_some_and(|value| value.len() > 4_194_304)
            || resolution.len() > 1_048_576
        {
            bail!("incoming ambiguity exceeds resource limits");
        }
        let memory_ids: Vec<String> = serde_json::from_str(&memory_ids)?;
        if memory_ids.len() > 1_000 {
            bail!("incoming ambiguity memory_ids cannot exceed 1000 entries");
        }
        let memory_ids = memory_ids
            .into_iter()
            .map(|id| {
                memory_id_map
                    .get(&id)
                    .cloned()
                    .unwrap_or_else(|| namespaced_unmapped_id(incoming_store, &id))
            })
            .collect::<Vec<_>>();
        conn.execute(
            "INSERT INTO ambiguities
             (uid, query, memory_ids, context, resolution, created_at, resolved_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                uid,
                query,
                serde_json::to_string(&memory_ids)?,
                context,
                resolution,
                created_at,
                resolved_at
            ],
        )?;
        let local_id = conn.last_insert_rowid();
        id_map.insert(incoming_id, local_id);
        report.ambiguities_imported += 1;
    }
    Ok((id_map, report))
}
