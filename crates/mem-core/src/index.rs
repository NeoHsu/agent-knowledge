use anyhow::{anyhow, Context, Result};
use rusqlite::Connection;

use crate::app::App;
use crate::db::{self, all_memories, memory_by_id, Memory};
use crate::search_index::{self, IndexedMemory};

/// Returns true if the index is stale, using only the SQLite metadata key.
/// The filesystem `.stale` marker is no longer used.
pub fn is_stale(app: &App) -> bool {
    dirty_in_db(app).unwrap_or(false)
}

pub fn mark_stale(app: &App, reason: &str) -> Result<()> {
    // reason is logged for observability but not persisted to a file
    let _ = reason;
    set_dirty(app, true)
}

pub fn clear_stale(app: &App) -> Result<()> {
    set_dirty(app, false)
}

pub fn set_dirty(app: &App, dirty: bool) -> Result<()> {
    let conn = app.conn()?;
    db::set_index_dirty(&conn, dirty)
}

pub fn dirty_in_db(app: &App) -> Result<bool> {
    let conn = app.conn()?;
    db::index_dirty(&conn)
}

pub fn reindex_or_mark_stale(app: &App, action: &str) -> Result<()> {
    match reindex(app) {
        Ok(()) => Ok(()),
        Err(err) => {
            let reason = format!("{action}: {err:#}");
            mark_stale(app, &reason).context("mark index stale")?;
            Err(err).with_context(|| {
                format!("{action}; index marked stale; run `mem reindex` or retry the query")
            })
        }
    }
}

pub fn upsert_or_mark_stale(app: &App, conn: &Connection, id: &str) -> Result<()> {
    match upsert(app, conn, id) {
        Ok(()) => Ok(()),
        Err(err) => {
            let action = format!("update index for {id}");
            let reason = format!("{action}: {err:#}");
            mark_stale(app, &reason).context("mark index stale")?;
            Err(err).with_context(|| {
                format!("{action}; index marked stale; run `mem reindex` or retry the query")
            })
        }
    }
}

pub fn upsert_batch_or_mark_stale(app: &App, conn: &Connection, ids: &[String]) -> Result<()> {
    if ids.is_empty() {
        return Ok(());
    }
    let memories: Vec<IndexedMemory> = ids
        .iter()
        .filter_map(|id| {
            memory_by_id(conn, id)
                .ok()
                .flatten()
                .map(|m| indexed_memory(&m))
        })
        .collect();
    match search_index::upsert_batch(&app.index_path, &memories) {
        Ok(()) => Ok(()),
        Err(err) => {
            mark_stale(app, &format!("batch upsert: {err:#}")).context("mark index stale")?;
            Err(err).context("batch index update failed; index marked stale; run `mem reindex`")
        }
    }
}

pub fn repair_stale(app: &App) -> Result<()> {
    if !is_stale(app) {
        return Ok(());
    }
    reindex_or_mark_stale(app, "repair stale index")
}

pub fn reindex(app: &App) -> Result<()> {
    let conn = app.conn()?;
    let memories = all_memories(&conn)?
        .iter()
        .map(indexed_memory)
        .collect::<Vec<_>>();
    search_index::rebuild(&app.index_path, &memories)?;
    clear_stale(app)?;
    Ok(())
}

pub fn upsert(app: &App, conn: &Connection, id: &str) -> Result<()> {
    let memory =
        memory_by_id(conn, id)?.ok_or_else(|| anyhow!("memory not found for index: {id}"))?;
    search_index::upsert(&app.index_path, &indexed_memory(&memory))?;
    Ok(())
}

pub fn search_ids(
    app: &App,
    query: &str,
    fuzzy: bool,
    raw_query: bool,
    limit: usize,
    type_filter: Option<&str>,
    scope_filter: Option<&[String]>,
) -> Result<Vec<String>> {
    search_index::search(
        &app.index_path,
        query,
        fuzzy,
        raw_query,
        limit,
        type_filter,
        scope_filter,
    )
}

fn indexed_memory(memory: &Memory) -> IndexedMemory {
    IndexedMemory {
        id: memory.id.clone(),
        name: memory.name.clone(),
        description: memory.description.clone(),
        content: memory.content.clone(),
        tags: memory.tags.clone(),
        scope: memory.scope.clone(),
        r#type: memory.r#type.clone(),
    }
}
