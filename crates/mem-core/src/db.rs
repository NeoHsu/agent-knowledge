use anyhow::{bail, Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::{util::now, INDEX_DIRTY_KEY};

pub(crate) const SCHEMA_VERSION: i64 = 5;

pub fn supported_schema_version() -> i64 {
    SCHEMA_VERSION
}

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
    pub origin: String,
    pub origin_ref: Option<String>,
    pub user_confirmed_at: Option<String>,
}

mod ambiguity;
mod changelog;
mod introspection;
mod memory;
mod metadata;
mod migration;
mod reporting;
mod security;
mod workflow_runs;

pub use ambiguity::{
    add_ambiguity_record, ambiguity_by_id, ambiguity_rows, resolve_ambiguity_record,
};
pub use changelog::log_change;
pub use introspection::{column_exists, table_exists};
pub use memory::{
    active_expired_memories, active_memory_count, all_memories, all_memories_compatible,
    all_workflows, gc_candidate_memories, graph_memories, insert_memory_record,
    list_memories_filtered, memories_by_ids, memory_by_id, memory_by_name, memory_by_name_in_scope,
    memory_count, memory_is_active, ranked_prime_memories, resolve_memory_ref,
    resolve_memory_ref_in_scopes, unique_memory_id, update_memory_from_merge, workflow_by_ref,
    workflow_by_ref_in_scopes, ACTIVE_MEMORY_SQL,
};
pub use metadata::{
    ensure_store_id, index_dirty, new_event_uid, set_index_dirty, store_id, STORE_ID_KEY,
};
pub use migration::{migrate_schema, schema_compatibility_required};
pub use reporting::{grouped_count, query_json_rows};
pub use security::{redact_store_secrets, validate_store_schema_objects, validate_store_secrets};
pub use workflow_runs::{log_workflow_run, workflow_run_counts, workflow_run_stats};

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
            origin: "direct".to_string(),
            origin_ref: None,
            user_confirmed_at: Some("2026-05-27T00:00:00Z".to_string()),
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
        for key in [
            STORE_ID_KEY,
            INDEX_DIRTY_KEY,
            "graph_dirty",
            "graph_schema_version",
        ] {
            let count: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM metadata WHERE key = ?1",
                    [key],
                    |row| row.get(0),
                )
                .expect("metadata key count");
            assert_eq!(count, 1, "missing metadata key {key}");
        }
        for table in [
            "graph_nodes",
            "graph_edges",
            "graph_semantic_edges",
            "graph_semantic_edge_revisions",
        ] {
            let exists: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
                    params![table],
                    |row| row.get(0),
                )
                .expect("graph table exists");
            assert_eq!(exists, 1, "missing {table}");
        }
        assert!(memory_by_name(&conn, "legacy").expect("legacy").is_some());
        conn.execute(
            "INSERT INTO memories
            (id, type, name, content, tags, scope, source, confidence, protected)
            VALUES ('workflow_id', 'workflow', 'workflow_name', 'content', '[]', 'global', 'manual', 'high', 1)",
            [],
        )
        .expect("workflow type allowed");
        let missing_uid = conn.execute(
            "INSERT INTO ambiguities (query, memory_ids) VALUES ('query', '[]')",
            [],
        );
        assert!(missing_uid.is_err(), "durable event UID must be required");
    }

    #[test]
    fn migrates_pre_release_v4_graph_table_without_ambiguity_column() {
        let conn = initialized_conn();
        conn.execute_batch(
            "DROP TABLE graph_semantic_edges;
             CREATE TABLE graph_semantic_edges (
                 id TEXT PRIMARY KEY,
                 source_ref TEXT NOT NULL,
                 target_ref TEXT NOT NULL,
                 relation TEXT NOT NULL,
                 confidence TEXT NOT NULL CHECK (confidence IN ('EXTRACTED', 'INFERRED', 'AMBIGUOUS')),
                 status TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'pending', 'rejected', 'superseded')),
                 evidence TEXT NOT NULL,
                 rationale TEXT,
                 source_spans TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(source_spans) AND json_type(source_spans) = 'array'),
                 tags TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(tags) AND json_type(tags) = 'array'),
                 generated_by TEXT NOT NULL DEFAULT 'agent' CHECK (generated_by IN ('agent', 'manual', 'import')),
                 source TEXT NOT NULL DEFAULT 'agent' CHECK (source IN ('manual', 'agent', 'daily_retro', 'weekly_retro')),
                 created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                 updated_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                 valid_until DATETIME,
                 version INTEGER DEFAULT 1 CHECK (version >= 1)
             );
             PRAGMA user_version = 4;",
        )
        .expect("create pre-release v4 graph shape");

        migrate_schema(&conn).expect("migrate pre-release v4 schema");

        let columns = conn
            .prepare("PRAGMA table_info(graph_semantic_edges)")
            .expect("graph table info")
            .query_map([], |row| row.get::<_, String>(1))
            .expect("graph columns")
            .collect::<rusqlite::Result<Vec<_>>>()
            .expect("collect graph columns");
        assert!(columns.iter().any(|column| column == "ambiguity_id"));
        assert!(columns.iter().any(|column| column == "user_confirmed_at"));
        let version: i64 = conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .expect("schema version");
        assert_eq!(version, SCHEMA_VERSION);
    }

    #[test]
    fn migrates_legacy_user_source_to_manual() {
        let conn = Connection::open_in_memory().expect("open memory db");
        conn.execute_batch(
            "CREATE TABLE memories (
                id TEXT PRIMARY KEY,
                type TEXT NOT NULL,
                name TEXT NOT NULL UNIQUE,
                description TEXT,
                content TEXT,
                tags TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(tags) AND json_type(tags) = 'array'),
                scope TEXT DEFAULT 'global',
                source TEXT NOT NULL,
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
            VALUES ('legacy_user', 'preference', 'legacy_user', 'content', '[]', 'global', 'user', 'medium', 0, '2026-05-27T00:00:00Z', '2026-05-27T00:00:00Z');
            PRAGMA user_version = 1;",
        )
        .expect("create legacy schema");

        migrate_schema(&conn).expect("migrate schema");

        let source: String = conn
            .query_row(
                "SELECT source FROM memories WHERE id = 'legacy_user'",
                [],
                |row| row.get(0),
            )
            .expect("legacy source");
        assert_eq!(source, "manual");
    }

    #[test]
    fn index_dirty_round_trips_through_metadata() {
        let conn = initialized_conn();

        assert!(index_dirty(&conn).expect("migration marks index dirty"));
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
    fn memories_by_ids_fetches_batch_for_caller_ordering() {
        let conn = initialized_conn();
        insert_memory_record(&conn, &memory("first", "first")).expect("insert first");
        insert_memory_record(&conn, &memory("second", "second")).expect("insert second");

        let ids = vec![
            "second".to_string(),
            "missing".to_string(),
            "first".to_string(),
        ];
        let mut by_id = memories_by_ids(&conn, &ids).expect("memories by ids");
        let ordered = ids
            .iter()
            .filter_map(|id| by_id.remove(id).map(|memory| memory.id))
            .collect::<Vec<_>>();

        assert_eq!(ordered, vec!["second", "first"]);
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
