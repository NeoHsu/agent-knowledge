use std::collections::HashMap;

use super::*;

pub fn memory_by_name(conn: &Connection, name: &str) -> Result<Option<Memory>> {
    let mut stmt = conn.prepare("SELECT * FROM memories WHERE name = ?1")?;
    stmt.query_row(params![name], row_to_memory)
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
    if let Some(memory) = memory_by_id(conn, reference)? {
        return Ok(memory.id);
    }
    if let Some(memory) = memory_by_name(conn, reference)? {
        return Ok(memory.id);
    }
    bail!("memory not found: {reference}")
}

pub fn all_memories(conn: &Connection) -> Result<Vec<Memory>> {
    let mut stmt = conn.prepare("SELECT * FROM memories ORDER BY created_at DESC")?;
    let rows = stmt.query_map([], row_to_memory)?;
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
    scopes: Option<&[String]>,
    expired: bool,
) -> Result<Vec<Memory>> {
    let mut conditions: Vec<String> = Vec::new();
    let mut params_vec: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

    if !include_superseded && !expired {
        conditions.push("valid_until IS NULL".to_string());
    }
    if expired {
        conditions.push("expires_at IS NOT NULL".to_string());
        conditions.push("datetime(expires_at) < datetime('now')".to_string());
        conditions.push("valid_until IS NULL".to_string());
    }
    if let Some(t) = r#type {
        conditions.push(format!("type = ?{}", params_vec.len() + 1));
        params_vec.push(Box::new(t.to_string()));
    }
    if let Some(t) = tag {
        // JSON array membership check using SQLite json_each
        conditions.push(format!(
            "EXISTS (SELECT 1 FROM json_each(tags) WHERE value = ?{})",
            params_vec.len() + 1
        ));
        params_vec.push(Box::new(t.to_string()));
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
                params_vec.push(Box::new(s.clone()));
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
    let rows = stmt.query_map(
        rusqlite::params_from_iter(params_vec.iter().map(|p| p.as_ref())),
        row_to_memory,
    )?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(Into::into)
}

pub fn memory_count(conn: &Connection) -> Result<usize> {
    let count: i64 = conn.query_row("SELECT COUNT(*) FROM memories", [], |row| row.get(0))?;
    Ok(count.max(0) as usize)
}

pub fn all_workflows(conn: &Connection, include_superseded: bool) -> Result<Vec<Memory>> {
    let sql = if include_superseded {
        "SELECT * FROM memories WHERE type = 'workflow' ORDER BY updated_at DESC"
    } else {
        "SELECT * FROM memories WHERE type = 'workflow' AND valid_until IS NULL ORDER BY updated_at DESC"
    };
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt.query_map([], row_to_memory)?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(Into::into)
}

pub fn workflow_by_ref(conn: &Connection, reference: &str) -> Result<Memory> {
    let memory_id = resolve_memory_ref(conn, reference)?;
    let Some(memory) = memory_by_id(conn, &memory_id)? else {
        bail!("memory not found: {reference}");
    };
    if memory.r#type != "workflow" {
        bail!("memory is not a workflow: {}", memory.name);
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
         created_at, updated_at, expires_at, valid_until, superseded_by, version, access_count, last_accessed_at)
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18)",
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
             version = version + 1
         WHERE id = ?13",
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
    })
}
