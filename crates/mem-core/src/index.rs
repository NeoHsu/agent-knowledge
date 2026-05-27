use anyhow::{anyhow, Context, Result};
use rusqlite::Connection;

use crate::app::App;
use crate::db::{self, all_memories, memory_by_id, Memory};
use crate::index_state;
use crate::search_index::{self, IndexedMemory};
use crate::util::now;

pub fn is_stale(app: &App) -> bool {
    index_state::is_stale(&app.index_path) || dirty_in_db(app).unwrap_or(false)
}

pub fn mark_stale(app: &App, reason: &str) -> Result<()> {
    index_state::mark_stale(&app.index_path, reason, &now())?;
    set_dirty(app, true)
}

pub fn clear_stale(app: &App) -> Result<()> {
    index_state::clear_stale(&app.index_path)?;
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

pub fn repair_stale(app: &App) -> Result<()> {
    if !is_stale(app) {
        return Ok(());
    }
    eprintln!("index is stale; rebuilding before search");
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
) -> Result<Vec<String>> {
    search_index::search(&app.index_path, query, fuzzy, raw_query, limit)
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
