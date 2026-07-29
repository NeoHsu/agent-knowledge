use std::collections::HashMap;

use super::*;
use crate::error;

pub const ACTIVE_MEMORY_SQL: &str =
    "valid_until IS NULL AND (expires_at IS NULL OR datetime(expires_at) >= datetime('now'))";

pub fn memory_is_active(memory: &Memory) -> bool {
    memory.valid_until.is_none() && !crate::util::is_expired(memory.expires_at.as_deref())
}

pub fn memory_by_name(conn: &Connection, name: &str) -> Result<Option<Memory>> {
    let mut stmt = conn.prepare("SELECT * FROM memories WHERE name = ?1 ORDER BY scope LIMIT 2")?;
    let rows = stmt.query_map(params![name], row_to_memory)?;
    let memories = rows.collect::<rusqlite::Result<Vec<_>>>()?;
    match memories.as_slice() {
        [] => Ok(None),
        [memory] => Ok(Some(memory.clone())),
        _ => Err(error::conflict(format!(
            "memory name is ambiguous across scopes: {name}; pass --scope or use id:<memory-id>"
        ))),
    }
}

pub fn memory_by_name_in_scope(
    conn: &Connection,
    name: &str,
    scope: &str,
) -> Result<Option<Memory>> {
    let mut stmt = conn.prepare("SELECT * FROM memories WHERE name = ?1 AND scope = ?2")?;
    stmt.query_row(params![name, scope], row_to_memory)
        .optional()
        .map_err(Into::into)
}

pub fn memory_by_id(conn: &Connection, id: &str) -> Result<Option<Memory>> {
    let mut stmt = conn.prepare("SELECT * FROM memories WHERE id = ?1")?;
    stmt.query_row(params![id], row_to_memory)
        .optional()
        .map_err(Into::into)
}

pub fn memories_by_ids(conn: &Connection, ids: &[String]) -> Result<HashMap<String, Memory>> {
    if ids.is_empty() {
        return Ok(HashMap::new());
    }

    let placeholders = (1..=ids.len())
        .map(|i| format!("?{i}"))
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!("SELECT * FROM memories WHERE id IN ({placeholders})");
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(
        rusqlite::params_from_iter(ids.iter().map(String::as_str)),
        row_to_memory,
    )?;
    let memories = rows.collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(memories
        .into_iter()
        .map(|memory| (memory.id.clone(), memory))
        .collect())
}

pub fn resolve_memory_ref(conn: &Connection, reference: &str) -> Result<String> {
    resolve_memory_ref_in_scopes(conn, reference, None)
}

pub fn resolve_memory_ref_in_scopes(
    conn: &Connection,
    reference: &str,
    scopes: Option<&[&str]>,
) -> Result<String> {
    if let Some(id) = reference.strip_prefix("id:") {
        return memory_by_id(conn, id)?
            .map(|memory| memory.id)
            .ok_or_else(|| error::not_found(format!("memory id not found: {id}")));
    }
    let Some(scopes) = scopes else {
        if let Some(memory) = memory_by_id(conn, reference)? {
            return Ok(memory.id);
        }
        return memory_by_name(conn, reference)?
            .map(|memory| memory.id)
            .ok_or_else(|| error::not_found(format!("memory not found: {reference}")));
    };
    let mut project_matches = Vec::new();
    let mut global_match = None;
    for scope in scopes {
        if let Some(memory) = memory_by_name_in_scope(conn, reference, scope)? {
            if *scope == "global" {
                global_match = Some(memory);
            } else {
                project_matches.push(memory);
            }
        }
    }
    match project_matches.as_slice() {
        [memory] => Ok(memory.id.clone()),
        [] => {
            if let Some(memory) = global_match {
                return Ok(memory.id);
            }
            memory_by_id(conn, reference)?
                .map(|memory| memory.id)
                .ok_or_else(|| error::not_found(format!("memory not found: {reference}")))
        }
        _ => Err(error::conflict(format!(
            "memory name is ambiguous across project scopes: {reference}; use id:<memory-id>"
        ))),
    }
}

pub fn all_memories(conn: &Connection) -> Result<Vec<Memory>> {
    let mut stmt = conn.prepare("SELECT * FROM memories ORDER BY created_at DESC")?;
    let rows = stmt.query_map([], row_to_memory)?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(Into::into)
}

pub fn all_memories_compatible(conn: &Connection) -> Result<Vec<Memory>> {
    let origin = if memory_column_exists(conn, "origin")? {
        "origin"
    } else {
        "'migration' AS origin"
    };
    let origin_ref = if memory_column_exists(conn, "origin_ref")? {
        "origin_ref"
    } else {
        "NULL AS origin_ref"
    };
    let user_confirmed_at = if memory_column_exists(conn, "user_confirmed_at")? {
        "user_confirmed_at"
    } else {
        "CASE WHEN source = 'manual' THEN created_at ELSE NULL END AS user_confirmed_at"
    };
    let sql = format!(
        "SELECT id, type, name, description, content, tags, COALESCE(scope, 'global') AS scope,
                source, confidence, protected, created_at, updated_at, expires_at, valid_until,
                superseded_by, version, access_count, last_accessed_at,
                {origin}, {origin_ref}, {user_confirmed_at}
         FROM memories ORDER BY created_at DESC"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([], row_to_memory)?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(Into::into)
}

fn memory_column_exists(conn: &Connection, column: &str) -> Result<bool> {
    let mut stmt = conn.prepare("PRAGMA table_info(memories)")?;
    let columns = stmt
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(columns.iter().any(|candidate| candidate == column))
}

/// Memories eligible for graph materialization. Active, unexpired rows are
/// included together with superseded tombstones needed to preserve lineage.
pub fn graph_memories(conn: &Connection) -> Result<Vec<Memory>> {
    let mut stmt = conn.prepare(
        "SELECT * FROM memories
         WHERE (
             valid_until IS NULL
             AND (expires_at IS NULL OR datetime(expires_at) >= datetime('now'))
         )
         OR superseded_by IS NOT NULL
         ORDER BY created_at DESC",
    )?;
    let rows = stmt.query_map([], row_to_memory)?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(Into::into)
}

/// Fetch the highest-priority active memories for one Prime section without
/// decoding or sorting the entire matching section in process.
pub fn ranked_prime_memories(
    conn: &Connection,
    memory_type: &str,
    scopes: &[&str],
    limit: usize,
) -> Result<Vec<Memory>> {
    if scopes.is_empty() || limit == 0 {
        return Ok(Vec::new());
    }
    let placeholders = (2..scopes.len() + 2)
        .map(|index| format!("?{index}"))
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!(
        "SELECT * FROM memories
         WHERE {ACTIVE_MEMORY_SQL}
           AND type = ?1
           AND scope IN ({placeholders})
         ORDER BY
           CASE WHEN scope = 'global' THEN 0 ELSE 1 END DESC,
           CASE source
             WHEN 'manual' THEN 4
             WHEN 'agent' THEN 3
             WHEN 'daily_retro' THEN 2
             WHEN 'weekly_retro' THEN 1
             ELSE 2
           END DESC,
           CASE confidence
             WHEN 'high' THEN 3
             WHEN 'medium' THEN 2
             ELSE 1
           END DESC,
           access_count DESC,
           updated_at DESC,
           name ASC,
           id ASC
         LIMIT {limit}"
    );
    let mut values = Vec::with_capacity(scopes.len() + 1);
    values.push(memory_type);
    values.extend_from_slice(scopes);
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(rusqlite::params_from_iter(values), row_to_memory)?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(Into::into)
}

/// Query memories with optional filters pushed into the SQL WHERE clause to
/// avoid a full table scan followed by in-process filtering.
pub fn list_memories_filtered(
    conn: &Connection,
    include_superseded: bool,
    r#type: Option<&str>,
    tag: Option<&str>,
    scopes: Option<&[&str]>,
    expired: bool,
) -> Result<Vec<Memory>> {
    let mut conditions: Vec<String> = Vec::new();
    let mut params_vec: Vec<&str> = Vec::new();

    if expired {
        conditions.push("expires_at IS NOT NULL".to_string());
        conditions.push("datetime(expires_at) < datetime('now')".to_string());
        conditions.push("valid_until IS NULL".to_string());
    } else if include_superseded {
        conditions
            .push("(expires_at IS NULL OR datetime(expires_at) >= datetime('now'))".to_string());
    } else {
        conditions.push(ACTIVE_MEMORY_SQL.to_string());
    }
    if let Some(t) = r#type {
        conditions.push(format!("type = ?{}", params_vec.len() + 1));
        params_vec.push(t);
    }
    if let Some(t) = tag {
        // JSON array membership check using SQLite json_each
        conditions.push(format!(
            "EXISTS (SELECT 1 FROM json_each(tags) WHERE value = ?{})",
            params_vec.len() + 1
        ));
        params_vec.push(t);
    }
    if let Some(sc) = scopes {
        if !sc.is_empty() {
            let placeholders: Vec<String> = sc
                .iter()
                .enumerate()
                .map(|(i, _)| format!("?{}", params_vec.len() + i + 1))
                .collect();
            conditions.push(format!("scope IN ({})", placeholders.join(", ")));
            for s in sc {
                params_vec.push(*s);
            }
        }
    }

    let where_clause = if conditions.is_empty() {
        String::new()
    } else {
        format!("WHERE {}", conditions.join(" AND "))
    };

    let sql = format!(
        "SELECT * FROM memories {} ORDER BY created_at DESC",
        where_clause
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(rusqlite::params_from_iter(params_vec), row_to_memory)?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(Into::into)
}

pub fn active_memory_count(conn: &Connection) -> Result<usize> {
    let sql = format!("SELECT COUNT(*) FROM memories WHERE {ACTIVE_MEMORY_SQL}");
    let count: i64 = conn.query_row(&sql, [], |row| row.get(0))?;
    Ok(count.max(0) as usize)
}

pub fn memory_count(conn: &Connection) -> Result<usize> {
    let count: i64 = conn.query_row("SELECT COUNT(*) FROM memories", [], |row| row.get(0))?;
    Ok(count.max(0) as usize)
}

pub fn all_workflows(conn: &Connection, include_superseded: bool) -> Result<Vec<Memory>> {
    let sql = if include_superseded {
        "SELECT * FROM memories
         WHERE type = 'workflow'
           AND (expires_at IS NULL OR datetime(expires_at) >= datetime('now'))
         ORDER BY updated_at DESC"
    } else {
        "SELECT * FROM memories
         WHERE type = 'workflow'
           AND valid_until IS NULL
           AND (expires_at IS NULL OR datetime(expires_at) >= datetime('now'))
         ORDER BY updated_at DESC"
    };
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt.query_map([], row_to_memory)?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(Into::into)
}

pub fn workflow_by_ref(conn: &Connection, reference: &str) -> Result<Memory> {
    workflow_by_ref_in_scopes(conn, reference, None)
}

pub fn workflow_by_ref_in_scopes(
    conn: &Connection,
    reference: &str,
    scopes: Option<&[&str]>,
) -> Result<Memory> {
    let memory_id = resolve_memory_ref_in_scopes(conn, reference, scopes)?;
    let Some(memory) = memory_by_id(conn, &memory_id)? else {
        return Err(error::not_found(format!("memory not found: {reference}")));
    };
    if memory.r#type != "workflow" {
        return Err(error::conflict(format!(
            "memory is not a workflow: {}",
            memory.name
        )));
    }
    if !memory_is_active(&memory) {
        return Err(error::not_found(format!(
            "workflow is expired, deleted, or superseded: {}",
            memory.name
        )));
    }
    Ok(memory)
}

pub fn active_expired_memories(conn: &Connection) -> Result<Vec<Memory>> {
    let mut stmt = conn.prepare(
        "SELECT * FROM memories
         WHERE expires_at IS NOT NULL
         AND datetime(expires_at) < datetime('now')
         AND valid_until IS NULL",
    )?;
    let rows = stmt.query_map([], row_to_memory)?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(Into::into)
}

pub fn gc_candidate_memories(conn: &Connection, cutoff: &str) -> Result<Vec<Memory>> {
    let mut stmt = conn.prepare(
        "SELECT * FROM memories
         WHERE valid_until IS NOT NULL
         AND datetime(valid_until) < datetime(?1)",
    )?;
    let rows = stmt.query_map(params![cutoff], row_to_memory)?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(Into::into)
}

pub fn insert_memory_record(conn: &Connection, memory: &Memory) -> Result<()> {
    conn.execute(
        "INSERT INTO memories
        (id, type, name, description, content, tags, scope, source, confidence, protected,
         created_at, updated_at, expires_at, valid_until, superseded_by, version, access_count,
         last_accessed_at, origin, origin_ref, user_confirmed_at)
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15,
                ?16, ?17, ?18, ?19, ?20, ?21)",
        params![
            memory.id,
            memory.r#type,
            memory.name,
            memory.description,
            memory.content,
            memory.tags,
            memory.scope,
            memory.source,
            memory.confidence,
            memory.protected,
            memory.created_at,
            memory.updated_at,
            memory.expires_at,
            memory.valid_until,
            memory.superseded_by,
            memory.version,
            memory.access_count,
            memory.last_accessed_at,
            memory.origin,
            memory.origin_ref,
            memory.user_confirmed_at,
        ],
    )?;
    Ok(())
}

pub fn update_memory_from_merge(
    conn: &Connection,
    existing: &Memory,
    incoming: &Memory,
) -> Result<()> {
    let now = now();
    conn.execute(
        "UPDATE memories
         SET type = ?1, description = ?2, content = ?3, tags = ?4, scope = ?5,
             source = ?6, confidence = ?7, protected = ?8, updated_at = ?9,
             expires_at = ?10, valid_until = ?11, superseded_by = ?12,
             access_count = MAX(access_count, ?13),
             last_accessed_at = CASE
                 WHEN last_accessed_at IS NULL THEN ?14
                 WHEN ?14 IS NULL THEN last_accessed_at
                 WHEN datetime(?14) > datetime(last_accessed_at) THEN ?14
                 ELSE last_accessed_at
             END,
             origin = 'merge', origin_ref = ?15, user_confirmed_at = ?16,
             version = version + 1
         WHERE id = ?17",
        params![
            &incoming.r#type,
            &incoming.description,
            &incoming.content,
            &incoming.tags,
            &incoming.scope,
            &incoming.source,
            &incoming.confidence,
            incoming.protected,
            now,
            &incoming.expires_at,
            &incoming.valid_until,
            &incoming.superseded_by,
            incoming.access_count,
            &incoming.last_accessed_at,
            &incoming.origin_ref,
            &incoming.user_confirmed_at,
            &existing.id,
        ],
    )?;
    log_change(
        conn,
        &existing.id,
        "merge",
        existing.content.as_deref(),
        incoming.content.as_deref(),
        "merge",
    )?;
    Ok(())
}

pub fn unique_memory_id(conn: &Connection, preferred: &str) -> Result<String> {
    let base = if preferred.trim().is_empty() {
        format!("memory_{}", uuid::Uuid::new_v4())
    } else {
        preferred.to_string()
    };
    if memory_by_id(conn, &base)?.is_none() {
        return Ok(base);
    }
    for suffix in 2.. {
        let candidate = format!("{base}_{suffix}");
        if memory_by_id(conn, &candidate)?.is_none() {
            return Ok(candidate);
        }
    }
    unreachable!()
}

fn row_to_memory(row: &rusqlite::Row<'_>) -> rusqlite::Result<Memory> {
    Ok(Memory {
        id: row.get("id")?,
        r#type: row.get("type")?,
        name: row.get("name")?,
        description: row.get("description")?,
        content: row.get("content")?,
        tags: row.get("tags")?,
        scope: row.get("scope")?,
        source: row.get("source")?,
        confidence: row.get("confidence")?,
        protected: row.get("protected")?,
        created_at: row.get("created_at")?,
        updated_at: row.get("updated_at")?,
        expires_at: row.get("expires_at")?,
        valid_until: row.get("valid_until")?,
        superseded_by: row.get("superseded_by")?,
        version: row.get("version")?,
        access_count: row.get("access_count")?,
        last_accessed_at: row.get("last_accessed_at")?,
        origin: row.get("origin")?,
        origin_ref: row.get("origin_ref")?,
        user_confirmed_at: row.get("user_confirmed_at")?,
    })
}
