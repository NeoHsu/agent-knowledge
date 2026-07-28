use std::collections::HashMap;

use super::events::{compatible_uid_expression, effective_event_uid, local_event_id};
use super::report::DurableEventMergeReport;
use super::*;

pub(super) fn merge_semantic_revisions(
    conn: &Connection,
    incoming: &Connection,
    incoming_store: &str,
    edge_id_map: &HashMap<String, String>,
    memory_id_map: &HashMap<String, String>,
    allow_secret_redaction: bool,
    report: &mut DurableEventMergeReport,
) -> Result<()> {
    if !mem_core::db::table_exists(incoming, "graph_semantic_edge_revisions")? {
        return Ok(());
    }
    let uid_expr = compatible_uid_expression(incoming, "graph_semantic_edge_revisions")?;
    let sql = format!(
        "SELECT id, {uid_expr}, edge_id, version, action, snapshot, source, created_at
         FROM graph_semantic_edge_revisions ORDER BY id"
    );
    let mut stmt = incoming.prepare(&sql)?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, Option<String>>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, i64>(3)?,
            row.get::<_, String>(4)?,
            row.get::<_, String>(5)?,
            row.get::<_, Option<String>>(6)?,
            row.get::<_, String>(7)?,
        ))
    })?;
    for row in rows {
        let (id, uid, edge_id, version, action, snapshot, source, created_at) = row?;
        let uid = effective_event_uid(uid, incoming_store, "semantic-revision", id);
        if local_event_id(conn, "graph_semantic_edge_revisions", &uid)?.is_some() {
            report.semantic_revisions_identical += 1;
            continue;
        }
        let Some(edge_id) = edge_id_map.get(&edge_id) else {
            report.semantic_revisions_unresolved += 1;
            continue;
        };
        let snapshot = sanitize_secret_field(
            &snapshot,
            "semantic revision snapshot",
            allow_secret_redaction,
        )?;
        if snapshot.len() > 1_048_576 {
            bail!("incoming semantic revision snapshot exceeds 1048576 bytes");
        }
        let mut snapshot_value: Value = serde_json::from_str(&snapshot)?;
        remap_snapshot_memory_refs(&mut snapshot_value, memory_id_map);
        conn.execute(
            "INSERT INTO graph_semantic_edge_revisions
             (uid, edge_id, version, action, snapshot, source, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                uid,
                edge_id,
                version.max(1),
                action,
                serde_json::to_string(&snapshot_value)?,
                source,
                created_at
            ],
        )?;
        report.semantic_revisions_imported += 1;
    }
    Ok(())
}

fn remap_snapshot_memory_refs(value: &mut Value, memory_id_map: &HashMap<String, String>) {
    let Some(object) = value.as_object_mut() else {
        return;
    };
    for key in ["source_ref", "target_ref"] {
        let Some(reference) = object
            .get_mut(key)
            .and_then(|value| value.as_str())
            .map(str::to_string)
        else {
            continue;
        };
        let Some(id) = reference.strip_prefix("memory:") else {
            continue;
        };
        if let Some(mapped) = memory_id_map.get(id) {
            object.insert(key.to_string(), json!(format!("memory:{mapped}")));
        }
    }
}
