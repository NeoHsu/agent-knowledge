use anyhow::{Context, Result};
use chrono::DateTime;
use rusqlite::Connection;

use crate::app::App;
use crate::db::{self, all_memories, memories_by_ids, memory_by_id, Memory};
use crate::error;
use crate::search_index::{self, IndexedMemory};
use crate::util::parse_string_array;

const INDEX_UPSERT_CHUNK: usize = 500;

#[derive(Debug, Clone)]
pub struct MemorySearchHit {
    pub id: String,
    pub score: f64,
}

#[derive(Debug, Clone, Copy, Default)]
pub enum SearchLifecycle {
    #[default]
    Active,
    IncludeSuperseded,
    Expired,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct SearchFilters<'a> {
    pub memory_type: Option<&'a str>,
    pub scopes: Option<&'a [&'a str]>,
    pub tag: Option<&'a str>,
    pub lifecycle: SearchLifecycle,
}

/// Returns true if the index is stale, using only the SQLite metadata key.
/// The filesystem `.stale` marker is no longer used.
pub fn is_stale(app: &App) -> bool {
    dirty_in_db(app).unwrap_or(false)
}

pub fn validate_physical_index(app: &App) -> Result<()> {
    search_index::validate_existing(&app.index_path)
}

pub fn is_compatibility_error(error: &anyhow::Error) -> bool {
    search_index::is_compatibility_error(error)
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
    let conn = app.read_conn()?;
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
    if is_stale(app) || validate_physical_index(app).is_err() {
        return reindex_or_mark_stale(app, "rebuild stale or missing index after write");
    }
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
    if is_stale(app) || validate_physical_index(app).is_err() {
        return reindex_or_mark_stale(app, "rebuild stale or missing index after batch write");
    }
    write_batch(app, conn, ids)
}

/// Complete a bulk SQLite write that marked the index dirty before commit.
/// A previously stale/missing index is rebuilt; otherwise one batch upsert is
/// committed and the transactional dirty marker is cleared afterward.
pub fn complete_bulk_write(
    app: &App,
    conn: &Connection,
    ids: &[String],
    rebuild_required: bool,
) -> Result<()> {
    if ids.is_empty() {
        return Ok(());
    }
    if rebuild_required || validate_physical_index(app).is_err() {
        return reindex_or_mark_stale(app, "rebuild stale or missing index after bulk write");
    }
    write_batch(app, conn, ids)?;
    db::set_index_dirty(conn, false)
}

fn write_batch(app: &App, conn: &Connection, ids: &[String]) -> Result<()> {
    let batches = ids.chunks(INDEX_UPSERT_CHUNK).map(|batch| {
        let stored = memories_by_ids(conn, batch)?;
        batch
            .iter()
            .map(|id| {
                stored.get(id).map(indexed_memory).ok_or_else(|| {
                    error::integrity(format!("memory not found for batch index update: {id}"))
                })
            })
            .collect::<Result<Vec<_>>>()
    });
    match search_index::upsert_batches(&app.index_path, batches) {
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
    let memory = memory_by_id(conn, id)?
        .ok_or_else(|| error::integrity(format!("memory not found for index: {id}")))?;
    search_index::upsert(&app.index_path, &indexed_memory(&memory))?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub fn search_ids(
    app: &App,
    query: &str,
    fuzzy: bool,
    raw_query: bool,
    limit: usize,
    filters: SearchFilters<'_>,
    allow_repair: bool,
) -> Result<Vec<String>> {
    search_hits(app, query, fuzzy, raw_query, limit, filters, allow_repair)
        .map(|hits| hits.into_iter().map(|hit| hit.id).collect())
}

#[allow(clippy::too_many_arguments)]
pub fn search_hits(
    app: &App,
    query: &str,
    fuzzy: bool,
    raw_query: bool,
    limit: usize,
    filters: SearchFilters<'_>,
    allow_repair: bool,
) -> Result<Vec<MemorySearchHit>> {
    let search =
        || search_index::search_hits(&app.index_path, query, fuzzy, raw_query, limit, filters);
    let hits = match search() {
        Ok(hits) => hits,
        Err(err) if search_index::is_compatibility_error(&err) && allow_repair => {
            mark_stale(app, &format!("index compatibility: {err:#}"))
                .context("mark index stale")?;
            reindex(app).map_err(|rebuild_err| {
                error::compatibility(format!(
                    "index schema version mismatch; explicit rebuild failed; run `mem reindex`: {rebuild_err:#}"
                ))
            })?;
            search()?
        }
        Err(err) if search_index::is_compatibility_error(&err) => {
            return Err(error::compatibility(format!(
                "index schema version mismatch; read-only query will not rebuild it. \
                 Run `mem reindex` or retry with --repair-index: {err:#}"
            )));
        }
        Err(err) => return Err(err),
    };
    Ok(hits
        .into_iter()
        .map(|hit| MemorySearchHit {
            id: hit.id,
            score: hit.score,
        })
        .collect())
}

fn indexed_memory(memory: &Memory) -> IndexedMemory {
    let expires_at = match memory.expires_at.as_deref() {
        None => i64::MAX,
        Some(value) => DateTime::parse_from_rfc3339(value)
            .map(|timestamp| timestamp.timestamp())
            .unwrap_or(i64::MIN),
    };
    IndexedMemory {
        id: memory.id.clone(),
        name: memory.name.clone(),
        description: memory.description.clone(),
        content: memory.content.clone(),
        tags: memory.tags.clone(),
        exact_tags: parse_string_array(&memory.tags).unwrap_or_default(),
        scope: memory.scope.clone(),
        r#type: memory.r#type.clone(),
        valid: memory.valid_until.is_none(),
        expires_at,
    }
}
