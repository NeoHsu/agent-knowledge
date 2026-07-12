use std::collections::{HashMap, HashSet};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::PathBuf;
use std::time::Duration;

#[cfg(unix)]
use std::os::unix::fs::DirBuilderExt;

use rusqlite::OptionalExtension;

use super::*;

#[derive(Debug, Default, serde::Serialize)]
struct DurableEventMergeReport {
    ambiguities_imported: usize,
    ambiguities_identical: usize,
    workflow_runs_imported: usize,
    workflow_runs_identical: usize,
    workflow_runs_unresolved: usize,
    changelog_imported: usize,
    changelog_identical: usize,
    changelog_unresolved: usize,
    semantic_revisions_imported: usize,
    semantic_revisions_identical: usize,
    semantic_revisions_unresolved: usize,
}

pub(crate) fn cmd_merge(app: &App, args: MergeArgs) -> Result<()> {
    let (source_db, temporary_root) = if args.redact_secrets {
        let (database, root) = redacted_merge_snapshot(&args.db)?;
        (database, Some(root))
    } else {
        (args.db.clone(), None)
    };
    let result = merge_database(app, &source_db, args.prefer_trusted, args.redact_secrets);
    if let Some(root) = temporary_root {
        fs::remove_dir_all(root).ok();
    }
    print_write_json_pretty(app, result?)?;
    Ok(())
}

pub(crate) fn merge_database(
    app: &App,
    db: &Path,
    prefer_trusted: bool,
    allow_secret_redaction: bool,
) -> Result<Value> {
    app.require_schema()?;
    if !db.exists() {
        bail!("merge database not found: {}", db.display());
    }
    let merge_bytes = fs::metadata(db)?.len();
    if merge_bytes > 4_294_967_296 {
        bail!("merge database exceeds 4294967296 bytes");
    }

    let conn = app.conn()?;
    let theirs = Connection::open_with_flags(db, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
        .with_context(|| format!("open merge database {} read-only", db.display()))?;
    let incoming_schema: i64 = theirs.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    let supported_schema = supported_schema_version();
    if !(1..=supported_schema).contains(&incoming_schema) {
        bail!(
            "merge database schema v{incoming_schema} is unsupported; expected v1 through v{supported_schema}"
        );
    }
    let quick_check: String = theirs.query_row("PRAGMA quick_check", [], |row| row.get(0))?;
    if quick_check != "ok" {
        bail!("merge database failed SQLite quick_check: {quick_check}");
    }
    if !allow_secret_redaction {
        mem_core::db::validate_store_secrets(&theirs)?;
    }
    let incoming_store = incoming_store_key(&theirs)?;
    let incoming = all_memories_compatible(&theirs)?;
    let mut imported = 0;
    let mut identical = 0;
    let mut conflicts = 0;
    let mut trusted_updates = 0;
    let mut rejected_lower_trust = 0;
    let mut regenerated_ids = 0;
    let mut workflow_review_required = 0;
    let mut unattested_manual_downgraded = 0;
    let mut changed_index_ids = Vec::new();
    let mut memory_id_map = HashMap::new();
    let mut review_memory_ids = HashSet::new();

    let (semantic_merge, durable_events) = with_transaction(&conn, |conn| {
        for mut memory in incoming {
            sanitize_incoming_memory(&mut memory, &incoming_store, allow_secret_redaction)?;
            if memory.source == "manual" && memory.user_confirmed_at.is_none() {
                memory.source = "agent".to_string();
                memory.confidence = "medium".to_string();
                memory.protected = false;
                unattested_manual_downgraded += 1;
            }
            if let Err(err) = workflow_core::validate_record_content(&memory) {
                add_workflow_review_record(conn, db, &memory, &err)?;
                workflow_review_required += 1;
                continue;
            }

            if let Some(existing) = memory_by_name_in_scope(conn, &memory.name, &memory.scope)? {
                memory_id_map.insert(memory.id.clone(), existing.id.clone());
                if normalized_text(existing.content.as_deref().unwrap_or_default())
                    == normalized_text(memory.content.as_deref().unwrap_or_default())
                {
                    merge_memory_usage(conn, &existing, &memory)?;
                    identical += 1;
                    continue;
                }

                let incoming_priority = source_priority(&memory.source);
                let existing_priority = source_priority(&existing.source);
                if incoming_priority < existing_priority {
                    review_memory_ids.insert(memory.id.clone());
                    rejected_lower_trust += 1;
                    continue;
                }
                if prefer_trusted && incoming_priority > existing_priority {
                    update_memory_from_merge(conn, &existing, &memory)?;
                    changed_index_ids.push(existing.id.clone());
                    trusted_updates += 1;
                    continue;
                }

                review_memory_ids.insert(memory.id.clone());
                let context = serde_json::to_string(&json!({
                    "kind": "merge_conflict",
                    "source_store": incoming_store,
                    "local": {
                        "id": &existing.id,
                        "name": &existing.name,
                        "scope": &existing.scope,
                        "source": &existing.source,
                        "priority": existing_priority,
                        "content": &existing.content
                    },
                    "incoming": {
                        "id": &memory.id,
                        "name": &memory.name,
                        "type": &memory.r#type,
                        "description": &memory.description,
                        "content": &memory.content,
                        "tags": &memory.tags,
                        "scope": &memory.scope,
                        "source": &memory.source,
                        "confidence": &memory.confidence,
                        "priority": incoming_priority,
                        "version": memory.version
                    }
                }))?;
                add_ambiguity_record(
                    conn,
                    &format!("merge:{}:{}", memory.scope, memory.name),
                    &[existing.id.clone(), memory.id.clone()],
                    Some(&context),
                )?;
                conflicts += 1;
                continue;
            }

            let original_id = memory.id.clone();
            memory.id = unique_memory_id(conn, &memory.id)?;
            if memory.id != original_id {
                regenerated_ids += 1;
            }
            memory_id_map.insert(original_id, memory.id.clone());
            insert_memory_record(conn, &memory)?;
            log_change(
                conn,
                &memory.id,
                "merge",
                None,
                memory.content.as_deref(),
                "merge",
            )?;
            changed_index_ids.push(memory.id.clone());
            imported += 1;
        }

        let (ambiguity_id_map, mut durable_events) = merge_ambiguities(
            conn,
            &theirs,
            &incoming_store,
            &memory_id_map,
            allow_secret_redaction,
        )?;
        let semantic_merge = mem_core::graph::merge_semantic_edges(
            conn,
            &theirs,
            &memory_id_map,
            &ambiguity_id_map,
            &review_memory_ids,
            prefer_trusted,
            allow_secret_redaction,
        )?;
        merge_workflow_runs(
            conn,
            &theirs,
            &incoming_store,
            &memory_id_map,
            allow_secret_redaction,
            &mut durable_events,
        )?;
        merge_changelog(
            conn,
            &theirs,
            &incoming_store,
            &memory_id_map,
            allow_secret_redaction,
            &mut durable_events,
        )?;
        merge_semantic_revisions(
            conn,
            &theirs,
            &incoming_store,
            &semantic_merge.edge_id_map,
            &memory_id_map,
            allow_secret_redaction,
            &mut durable_events,
        )?;

        if !changed_index_ids.is_empty()
            || conflicts > 0
            || workflow_review_required > 0
            || semantic_merge.changed()
        {
            mem_core::graph::set_graph_dirty(conn, true)?;
        }
        Ok((semantic_merge, durable_events))
    })?;

    memory_index::upsert_batch_or_mark_stale(app, &conn, &changed_index_ids)?;

    Ok(json!({
        "status": "merged",
        "source_store": incoming_store,
        "imported": imported,
        "identical": identical,
        "conflicts": conflicts,
        "trusted_updates": trusted_updates,
        "rejected_lower_trust": rejected_lower_trust,
        "unattested_manual_downgraded": unattested_manual_downgraded,
        "workflow_review_required": workflow_review_required,
        "regenerated_ids": regenerated_ids,
        "semantic_edges": semantic_merge,
        "durable_events": durable_events
    }))
}

fn redacted_merge_snapshot(source_path: &Path) -> Result<(PathBuf, PathBuf)> {
    if !source_path.is_file() {
        bail!("merge database not found: {}", source_path.display());
    }
    let root = std::env::temp_dir().join(format!(
        "mnemark-merge-redacted-{}",
        uuid::Uuid::new_v4().simple()
    ));
    let mut builder = fs::DirBuilder::new();
    #[cfg(unix)]
    builder.mode(0o700);
    builder
        .create(&root)
        .with_context(|| format!("create secure temporary directory {}", root.display()))?;
    let database = root.join("memory.db");
    let result = (|| -> Result<()> {
        let source =
            Connection::open_with_flags(source_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        let mut destination = Connection::open(&database)?;
        rusqlite::backup::Backup::new(&source, &mut destination)?.run_to_completion(
            5,
            Duration::from_millis(25),
            None,
        )?;
        mem_core::db::redact_store_secrets(&mut destination)?;
        Ok(())
    })();
    if let Err(error) = result {
        fs::remove_dir_all(&root).ok();
        return Err(error);
    }
    Ok((database, root))
}

fn sanitize_incoming_memory(
    memory: &mut Memory,
    incoming_store: &str,
    allow_secret_redaction: bool,
) -> Result<()> {
    memory.name = sanitize_secret_field(&memory.name, "memory name", allow_secret_redaction)?;
    memory.description = memory
        .description
        .as_deref()
        .map(|value| sanitize_secret_field(value, "memory description", allow_secret_redaction))
        .transpose()?;
    memory.content = memory
        .content
        .as_deref()
        .map(|value| sanitize_secret_field(value, "memory content", allow_secret_redaction))
        .transpose()?;
    memory.tags = sanitize_secret_field(&memory.tags, "memory tags", allow_secret_redaction)?;
    memory.scope = sanitize_secret_field(&memory.scope, "memory scope", allow_secret_redaction)?;
    validate_tags(&memory.tags)?;
    scope::validate_scope(&memory.scope)?;
    validate_memory_resource_limits(
        &memory.name,
        memory.description.as_deref(),
        memory.content.as_deref().unwrap_or_default(),
        &memory.tags,
        &memory.scope,
        None,
    )?;
    memory.origin = "merge".to_string();
    memory.origin_ref = Some(incoming_store.to_string());
    Ok(())
}

fn merge_memory_usage(conn: &Connection, existing: &Memory, incoming: &Memory) -> Result<()> {
    conn.execute(
        "UPDATE memories
         SET access_count = MAX(access_count, ?1),
             last_accessed_at = CASE
                 WHEN last_accessed_at IS NULL THEN ?2
                 WHEN ?2 IS NULL THEN last_accessed_at
                 WHEN datetime(?2) > datetime(last_accessed_at) THEN ?2
                 ELSE last_accessed_at
             END
         WHERE id = ?3",
        params![
            incoming.access_count,
            incoming.last_accessed_at,
            existing.id
        ],
    )?;
    Ok(())
}

fn merge_ambiguities(
    conn: &Connection,
    incoming: &Connection,
    incoming_store: &str,
    memory_id_map: &HashMap<String, String>,
    allow_secret_redaction: bool,
) -> Result<(HashMap<i64, i64>, DurableEventMergeReport)> {
    let mut report = DurableEventMergeReport::default();
    let mut id_map = HashMap::new();
    if !table_exists(incoming, "ambiguities")? {
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

fn merge_workflow_runs(
    conn: &Connection,
    incoming: &Connection,
    incoming_store: &str,
    memory_id_map: &HashMap<String, String>,
    allow_secret_redaction: bool,
    report: &mut DurableEventMergeReport,
) -> Result<()> {
    if !table_exists(incoming, "workflow_runs")? {
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

fn merge_changelog(
    conn: &Connection,
    incoming: &Connection,
    incoming_store: &str,
    memory_id_map: &HashMap<String, String>,
    allow_secret_redaction: bool,
    report: &mut DurableEventMergeReport,
) -> Result<()> {
    if !table_exists(incoming, "changelog")? {
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

fn merge_semantic_revisions(
    conn: &Connection,
    incoming: &Connection,
    incoming_store: &str,
    edge_id_map: &HashMap<String, String>,
    memory_id_map: &HashMap<String, String>,
    allow_secret_redaction: bool,
    report: &mut DurableEventMergeReport,
) -> Result<()> {
    if !table_exists(incoming, "graph_semantic_edge_revisions")? {
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

fn sanitize_optional(
    value: Option<&str>,
    field: &str,
    allow_secret_redaction: bool,
) -> Result<Option<String>> {
    value
        .map(|value| sanitize_secret_field(value, field, allow_secret_redaction))
        .transpose()
}

fn incoming_store_key(conn: &Connection) -> Result<String> {
    if table_exists(conn, "metadata")? {
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

fn compatible_uid_expression(conn: &Connection, table: &str) -> Result<&'static str> {
    if column_exists(conn, table, "uid")? {
        Ok("uid")
    } else {
        Ok("NULL AS uid")
    }
}

fn effective_event_uid(uid: Option<String>, incoming_store: &str, kind: &str, id: i64) -> String {
    uid.filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| format!("{incoming_store}:{kind}:legacy:{id}"))
}

fn local_event_id(conn: &Connection, table: &str, uid: &str) -> Result<Option<i64>> {
    conn.query_row(
        &format!("SELECT id FROM {table} WHERE uid = ?1"),
        params![uid],
        |row| row.get(0),
    )
    .optional()
    .map_err(Into::into)
}

fn namespaced_unmapped_id(incoming_store: &str, id: &str) -> String {
    format!("unmapped:{incoming_store}:{id}")
}

fn table_exists(conn: &Connection, table: &str) -> Result<bool> {
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1)",
        params![table],
        |row| row.get(0),
    )
    .map_err(Into::into)
}

fn column_exists(conn: &Connection, table: &str, column: &str) -> Result<bool> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let columns = stmt
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(columns.iter().any(|candidate| candidate == column))
}

fn add_workflow_review_record(
    conn: &Connection,
    source_db: &Path,
    memory: &Memory,
    err: &anyhow::Error,
) -> Result<()> {
    let context = serde_json::to_string(&json!({
        "kind": "workflow_validation_failed",
        "source_db": source_db.display().to_string(),
        "error": err.to_string(),
        "incoming": {
            "id": &memory.id,
            "name": &memory.name,
            "type": &memory.r#type,
            "description": &memory.description,
            "content": &memory.content,
            "tags": &memory.tags,
            "scope": &memory.scope,
            "source": &memory.source,
            "confidence": &memory.confidence,
            "version": memory.version
        },
        "review": {
            "action": "fix_or_reject_before_import",
            "reason": "workflow memories must be valid before merge can import or update them"
        }
    }))?;
    add_ambiguity_record(
        conn,
        &format!("merge workflow review:{}:{}", memory.scope, memory.name),
        std::slice::from_ref(&memory.id),
        Some(&context),
    )?;
    Ok(())
}
