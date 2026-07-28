use std::path::PathBuf;
use std::time::Duration;

#[cfg(unix)]
use std::os::unix::fs::DirBuilderExt;

use super::*;
use ambiguities::merge_ambiguities;
use events::{incoming_store_key, merge_changelog, merge_workflow_runs};
use memory::merge_memories;
use semantic::merge_semantic_revisions;

mod ambiguities;
mod events;
mod memory;
mod report;
mod sanitize;
mod semantic;

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

    let (memory_merge, semantic_merge, durable_events) = with_transaction(&conn, |conn| {
        let memory_merge = merge_memories(
            conn,
            db,
            &incoming_store,
            incoming,
            prefer_trusted,
            allow_secret_redaction,
        )?;
        let (ambiguity_id_map, mut durable_events) = merge_ambiguities(
            conn,
            &theirs,
            &incoming_store,
            &memory_merge.memory_id_map,
            allow_secret_redaction,
        )?;
        let semantic_merge = mem_core::graph::merge_semantic_edges(
            conn,
            &theirs,
            &memory_merge.memory_id_map,
            &ambiguity_id_map,
            &memory_merge.review_memory_ids,
            prefer_trusted,
            allow_secret_redaction,
        )?;
        merge_workflow_runs(
            conn,
            &theirs,
            &incoming_store,
            &memory_merge.memory_id_map,
            allow_secret_redaction,
            &mut durable_events,
        )?;
        merge_changelog(
            conn,
            &theirs,
            &incoming_store,
            &memory_merge.memory_id_map,
            allow_secret_redaction,
            &mut durable_events,
        )?;
        merge_semantic_revisions(
            conn,
            &theirs,
            &incoming_store,
            &semantic_merge.edge_id_map,
            &memory_merge.memory_id_map,
            allow_secret_redaction,
            &mut durable_events,
        )?;

        if !memory_merge.changed_index_ids.is_empty()
            || memory_merge.conflicts > 0
            || memory_merge.workflow_review_required > 0
            || semantic_merge.changed()
        {
            mem_core::graph::set_graph_dirty(conn, true)?;
        }
        Ok((memory_merge, semantic_merge, durable_events))
    })?;

    finish_committed_index_write(
        memory_index::upsert_batch_or_mark_stale(app, &conn, &memory_merge.changed_index_ids),
        "database merge",
        json!({
            "source_store": incoming_store,
            "changed_count": memory_merge.changed_index_ids.len()
        }),
    )?;

    Ok(json!({
        "status": "merged",
        "source_store": incoming_store,
        "imported": memory_merge.imported,
        "identical": memory_merge.identical,
        "conflicts": memory_merge.conflicts,
        "trusted_updates": memory_merge.trusted_updates,
        "rejected_lower_trust": memory_merge.rejected_lower_trust,
        "unattested_manual_downgraded": memory_merge.unattested_manual_downgraded,
        "workflow_review_required": memory_merge.workflow_review_required,
        "regenerated_ids": memory_merge.regenerated_ids,
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
    #[cfg(unix)]
    let builder = {
        let mut builder = fs::DirBuilder::new();
        builder.mode(0o700);
        builder
    };
    #[cfg(not(unix))]
    let builder = fs::DirBuilder::new();
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
