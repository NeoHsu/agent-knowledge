use anyhow::{bail, Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::{util::now, INDEX_DIRTY_KEY};

pub(crate) const SCHEMA_VERSION: i64 = 3;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Memory {
    pub id: String,
    pub r#type: String,
    pub name: String,
    pub description: Option<String>,
    pub content: Option<String>,
    pub tags: String,
    pub scope: String,
    pub source: String,
    pub confidence: String,
    pub protected: bool,
    pub created_at: String,
    pub updated_at: String,
    pub expires_at: Option<String>,
    pub valid_until: Option<String>,
    pub superseded_by: Option<String>,
    pub version: i64,
    pub access_count: i64,
    pub last_accessed_at: Option<String>,
}

pub fn migrate_schema(conn: &Connection) -> Result<()> {
    let version: i64 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    if version > SCHEMA_VERSION {
        bail!("database schema version {version} is newer than supported version {SCHEMA_VERSION}");
    }
    if version < 2 {
        migrate_memories_type_check_v2(conn)?;
    }
    if version < 3 {
        migrate_metadata_v3(conn)?;
    }
    conn.pragma_update(None, "user_version", SCHEMA_VERSION)?;
    Ok(())
}

pub fn with_transaction<T, F>(conn: &Connection, f: F) -> Result<T>
where
    F: FnOnce(&Connection) -> Result<T>,
{
    conn.execute_batch("BEGIN IMMEDIATE TRANSACTION;")?;
    let result = f(conn);
    match result {
        Ok(value) => {
            if let Err(err) = conn.execute_batch("COMMIT;") {
                let _ = conn.execute_batch("ROLLBACK;");
                Err(err.into())
            } else {
                Ok(value)
            }
        }
        Err(err) => {
            let _ = conn.execute_batch("ROLLBACK;");
            Err(err)
        }
    }
}

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
    let memory = memory_by_id(conn, &memory_id)?.expect("resolved memory exists");
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

pub fn log_change(
    conn: &Connection,
    memory_id: &str,
    action: &str,
    old_content: Option<&str>,
    new_content: Option<&str>,
    source: &str,
) -> Result<()> {
    conn.execute(
        "INSERT INTO changelog (memory_id, action, old_content, new_content, source)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![memory_id, action, old_content, new_content, source],
    )?;
    Ok(())
}

pub fn add_ambiguity_record(
    conn: &Connection,
    query: &str,
    memory_ids: &[String],
    context: Option<&str>,
) -> Result<()> {
    let memory_ids = serde_json::to_string(memory_ids)?;
    conn.execute(
        "INSERT INTO ambiguities (query, memory_ids, context, resolution)
         VALUES (?1, ?2, ?3, 'pending')",
        params![query, memory_ids, context],
    )?;
    Ok(())
}

pub fn grouped_count(conn: &Connection, column: &str) -> Result<Value> {
    let sql = format!(
        "SELECT {column}, COUNT(*) FROM memories WHERE valid_until IS NULL GROUP BY {column}"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
    })?;
    let mut map = serde_json::Map::new();
    for row in rows {
        let (key, count) = row?;
        map.insert(key, json!(count));
    }
    Ok(Value::Object(map))
}

pub fn query_json_rows(conn: &Connection, sql: &str) -> Result<Vec<Value>> {
    let mut stmt = conn.prepare(sql)?;
    let column_names: Vec<String> = stmt.column_names().iter().map(|s| s.to_string()).collect();
    let rows = stmt.query_map([], |row| {
        let mut map = serde_json::Map::new();
        for (idx, name) in column_names.iter().enumerate() {
            let value: rusqlite::types::Value = row.get(idx)?;
            map.insert(name.clone(), sqlite_value_to_json(value));
        }
        Ok(Value::Object(map))
    })?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(Into::into)
}

pub fn ambiguity_rows(conn: &Connection, pending_only: bool) -> Result<Vec<Value>> {
    let sql = if pending_only {
        "SELECT id, query, memory_ids, context, resolution, created_at, resolved_at
         FROM ambiguities
         WHERE resolution = 'pending'
         ORDER BY created_at DESC"
    } else {
        "SELECT id, query, memory_ids, context, resolution, created_at, resolved_at
         FROM ambiguities
         ORDER BY created_at DESC"
    };
    let mut rows = query_json_rows(conn, sql)?;
    for row in &mut rows {
        parse_json_string_field(row, "memory_ids");
        parse_json_string_field(row, "context");
        parse_json_string_field(row, "resolution");
    }
    Ok(rows)
}

pub fn ambiguity_by_id(conn: &Connection, id: i64) -> Result<Option<Value>> {
    conn.query_row(
        "SELECT id, query, memory_ids, context, resolution, created_at, resolved_at
         FROM ambiguities
         WHERE id = ?1",
        params![id],
        |row| {
            Ok(json!({
                "id": row.get::<_, i64>(0)?,
                "query": row.get::<_, String>(1)?,
                "memory_ids": row.get::<_, String>(2)?,
                "context": row.get::<_, Option<String>>(3)?,
                "resolution": row.get::<_, Option<String>>(4)?,
                "created_at": row.get::<_, String>(5)?,
                "resolved_at": row.get::<_, Option<String>>(6)?,
            }))
        },
    )
    .optional()
    .map_err(Into::into)
}

pub fn set_index_dirty(conn: &Connection, dirty: bool) -> Result<()> {
    conn.execute(
        "INSERT INTO metadata (key, value, updated_at)
         VALUES (?1, ?2, ?3)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
        params![INDEX_DIRTY_KEY, if dirty { "true" } else { "false" }, now()],
    )?;
    Ok(())
}

pub fn index_dirty(conn: &Connection) -> Result<bool> {
    let value: Option<String> = conn
        .query_row(
            "SELECT value FROM metadata WHERE key = ?1",
            params![INDEX_DIRTY_KEY],
            |row| row.get(0),
        )
        .optional()?;
    Ok(value.as_deref() == Some("true"))
}

fn migrate_memories_type_check_v2(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS memories_v2 (
            id TEXT PRIMARY KEY,
            type TEXT NOT NULL CHECK (type IN ('user', 'feedback', 'project', 'reference', 'preference', 'workflow')),
            name TEXT NOT NULL UNIQUE,
            description TEXT,
            content TEXT,
            tags TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(tags) AND json_type(tags) = 'array'),
            scope TEXT DEFAULT 'global',
            source TEXT NOT NULL CHECK (source IN ('manual', 'agent', 'daily_retro', 'weekly_retro')),
            confidence TEXT DEFAULT 'medium' CHECK (confidence IN ('high', 'medium', 'low')),
            protected BOOLEAN DEFAULT FALSE CHECK (protected IN (0, 1)),
            created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
            updated_at DATETIME DEFAULT CURRENT_TIMESTAMP,
            expires_at DATETIME,
            valid_until DATETIME,
            superseded_by TEXT,
            version INTEGER DEFAULT 1 CHECK (version >= 1),
            access_count INTEGER DEFAULT 0 CHECK (access_count >= 0),
            last_accessed_at DATETIME
        );
        INSERT INTO memories_v2
        SELECT id, type, name, description, content, tags, scope, source, confidence, protected,
               created_at, updated_at, expires_at, valid_until, superseded_by, version,
               access_count, last_accessed_at
        FROM memories;
        DROP TABLE memories;
        ALTER TABLE memories_v2 RENAME TO memories;
        CREATE INDEX IF NOT EXISTS idx_type ON memories(type);
        CREATE INDEX IF NOT EXISTS idx_scope ON memories(scope);
        CREATE INDEX IF NOT EXISTS idx_expires ON memories(expires_at);
        CREATE INDEX IF NOT EXISTS idx_source ON memories(source);
        CREATE INDEX IF NOT EXISTS idx_valid_until ON memories(valid_until);
        CREATE INDEX IF NOT EXISTS idx_access ON memories(access_count);
        CREATE INDEX IF NOT EXISTS idx_confidence ON memories(confidence);",
    )
    .context("migrate memories type constraint to schema v2")?;
    Ok(())
}

fn migrate_metadata_v3(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS metadata (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL,
            updated_at DATETIME DEFAULT CURRENT_TIMESTAMP
        );",
    )
    .context("create metadata table")?;
    Ok(())
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

fn parse_json_string_field(row: &mut Value, field: &str) {
    let Some(map) = row.as_object_mut() else {
        return;
    };
    let Some(raw) = map.get(field).and_then(Value::as_str) else {
        return;
    };
    if let Ok(parsed) = serde_json::from_str::<Value>(raw) {
        map.insert(field.to_string(), parsed);
    }
}

fn sqlite_value_to_json(value: rusqlite::types::Value) -> Value {
    match value {
        rusqlite::types::Value::Null => Value::Null,
        rusqlite::types::Value::Integer(v) => json!(v),
        rusqlite::types::Value::Real(v) => json!(v),
        rusqlite::types::Value::Text(v) => json!(v),
        rusqlite::types::Value::Blob(v) => json!(v),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn initialized_conn() -> Connection {
        let conn = Connection::open_in_memory().expect("open memory db");
        conn.execute_batch(include_str!("../../../schema/memory-schema.sql"))
            .expect("apply schema");
        migrate_schema(&conn).expect("migrate schema");
        conn
    }

    fn memory(id: &str, name: &str) -> Memory {
        Memory {
            id: id.to_string(),
            r#type: "reference".to_string(),
            name: name.to_string(),
            description: None,
            content: Some("content".to_string()),
            tags: "[]".to_string(),
            scope: "global".to_string(),
            source: "manual".to_string(),
            confidence: "high".to_string(),
            protected: true,
            created_at: "2026-05-27T00:00:00Z".to_string(),
            updated_at: "2026-05-27T00:00:00Z".to_string(),
            expires_at: None,
            valid_until: None,
            superseded_by: None,
            version: 1,
            access_count: 0,
            last_accessed_at: None,
        }
    }

    #[test]
    fn migrates_v1_database_to_current_schema() {
        let conn = Connection::open_in_memory().expect("open memory db");
        conn.execute_batch(
            "CREATE TABLE memories (
                id TEXT PRIMARY KEY,
                type TEXT NOT NULL CHECK (type IN ('user', 'feedback', 'project', 'reference', 'preference')),
                name TEXT NOT NULL UNIQUE,
                description TEXT,
                content TEXT,
                tags TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(tags) AND json_type(tags) = 'array'),
                scope TEXT DEFAULT 'global',
                source TEXT NOT NULL CHECK (source IN ('manual', 'agent', 'daily_retro', 'weekly_retro')),
                confidence TEXT DEFAULT 'medium' CHECK (confidence IN ('high', 'medium', 'low')),
                protected BOOLEAN DEFAULT FALSE CHECK (protected IN (0, 1)),
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                updated_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                expires_at DATETIME,
                valid_until DATETIME,
                superseded_by TEXT,
                version INTEGER DEFAULT 1 CHECK (version >= 1),
                access_count INTEGER DEFAULT 0 CHECK (access_count >= 0),
                last_accessed_at DATETIME
            );
            INSERT INTO memories
            (id, type, name, content, tags, scope, source, confidence, protected, created_at, updated_at)
            VALUES ('legacy', 'reference', 'legacy', 'content', '[]', 'global', 'manual', 'high', 1, '2026-05-27T00:00:00Z', '2026-05-27T00:00:00Z');
            PRAGMA user_version = 1;",
        )
        .expect("create v1 schema");

        migrate_schema(&conn).expect("migrate schema");

        let version: i64 = conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .expect("schema version");
        assert_eq!(version, SCHEMA_VERSION);
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM metadata", [], |row| row.get(0))
            .expect("metadata count");
        assert_eq!(count, 0);
        assert!(memory_by_name(&conn, "legacy").expect("legacy").is_some());
        conn.execute(
            "INSERT INTO memories
            (id, type, name, content, tags, scope, source, confidence, protected)
            VALUES ('workflow_id', 'workflow', 'workflow_name', 'content', '[]', 'global', 'manual', 'high', 1)",
            [],
        )
        .expect("workflow type allowed");
    }

    #[test]
    fn index_dirty_round_trips_through_metadata() {
        let conn = initialized_conn();

        assert!(!index_dirty(&conn).expect("initial dirty"));
        set_index_dirty(&conn, true).expect("set dirty");
        assert!(index_dirty(&conn).expect("dirty"));
        set_index_dirty(&conn, false).expect("clear dirty");
        assert!(!index_dirty(&conn).expect("clean"));
    }

    #[test]
    fn unique_memory_id_appends_numeric_suffix() {
        let conn = initialized_conn();
        insert_memory_record(&conn, &memory("release", "release")).expect("insert release");
        insert_memory_record(&conn, &memory("release_2", "release_2")).expect("insert release_2");

        let id = unique_memory_id(&conn, "release").expect("unique id");

        assert_eq!(id, "release_3");
    }

    #[test]
    fn ambiguity_rows_parse_json_string_fields() {
        let conn = initialized_conn();
        add_ambiguity_record(
            &conn,
            "merge:test",
            &["a".to_string(), "b".to_string()],
            Some(r#"{"kind":"merge_conflict"}"#),
        )
        .expect("add ambiguity");

        let rows = ambiguity_rows(&conn, true).expect("ambiguities");

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["memory_ids"], json!(["a", "b"]));
        assert_eq!(rows[0]["context"]["kind"], "merge_conflict");
    }
}
