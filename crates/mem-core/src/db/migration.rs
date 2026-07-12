use super::*;
use crate::graph::{GRAPH_DIRTY_KEY, GRAPH_SCHEMA_VERSION, GRAPH_SCHEMA_VERSION_KEY};

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
    if version < 4 {
        migrate_core_event_tables_v4(conn)?;
        migrate_graph_v4(conn)?;
    }
    if version < 5 {
        migrate_agent_memory_v5(conn)?;
    } else {
        repair_v5_compatibility(conn)?;
    }
    // Setting user_version rewrites the database header even when the value
    // is unchanged, which dirties memory.db byte-wise on every command and
    // would turn each `mem sync` into a spurious commit. Only write on upgrade.
    if version < SCHEMA_VERSION {
        conn.pragma_update(None, "user_version", SCHEMA_VERSION)?;
    }
    Ok(())
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
        SELECT id, type, name, description, content, tags, scope,
               CASE source WHEN 'user' THEN 'manual' ELSE source END,
               confidence, protected,
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

fn migrate_core_event_tables_v4(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS ambiguities (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            query TEXT NOT NULL,
            memory_ids TEXT NOT NULL CHECK (json_valid(memory_ids) AND json_type(memory_ids) = 'array'),
            context TEXT,
            resolution TEXT DEFAULT 'pending' CHECK (resolution = 'pending' OR json_valid(resolution)),
            created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
            resolved_at DATETIME
        );
        CREATE TABLE IF NOT EXISTS changelog (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            memory_id TEXT NOT NULL,
            action TEXT NOT NULL CHECK (action IN ('save', 'update', 'supersede', 'delete', 'merge', 'gc')),
            old_content TEXT,
            new_content TEXT,
            source TEXT,
            created_at DATETIME DEFAULT CURRENT_TIMESTAMP
        );
        CREATE TABLE IF NOT EXISTS workflow_runs (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            memory_id TEXT NOT NULL,
            result TEXT NOT NULL CHECK (result IN ('success', 'failure')),
            note TEXT,
            source TEXT,
            created_at DATETIME DEFAULT CURRENT_TIMESTAMP
        );",
    )
    .context("create durable event tables required by graph migration")?;
    Ok(())
}

fn migrate_graph_v4(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS graph_semantic_edges (
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
            user_confirmed_at DATETIME,
            created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
            updated_at DATETIME DEFAULT CURRENT_TIMESTAMP,
            valid_until DATETIME,
            ambiguity_id INTEGER,
            version INTEGER DEFAULT 1 CHECK (version >= 1),
            FOREIGN KEY(ambiguity_id) REFERENCES ambiguities(id)
        );

        CREATE TABLE IF NOT EXISTS graph_semantic_edge_revisions (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            edge_id TEXT NOT NULL,
            version INTEGER NOT NULL CHECK (version >= 1),
            action TEXT NOT NULL CHECK (action IN ('ingest', 'update', 'status', 'merge')),
            snapshot TEXT NOT NULL CHECK (json_valid(snapshot) AND json_type(snapshot) = 'object'),
            source TEXT,
            created_at DATETIME DEFAULT CURRENT_TIMESTAMP
        );

        CREATE TABLE IF NOT EXISTS graph_nodes (
            id TEXT PRIMARY KEY,
            kind TEXT NOT NULL,
            label TEXT NOT NULL,
            ref_table TEXT,
            ref_id TEXT,
            scope TEXT,
            metadata TEXT NOT NULL DEFAULT '{}' CHECK (json_valid(metadata) AND json_type(metadata) = 'object'),
            origin TEXT NOT NULL CHECK (origin IN ('deterministic', 'semantic')),
            created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
            updated_at DATETIME DEFAULT CURRENT_TIMESTAMP
        );

        CREATE TABLE IF NOT EXISTS graph_edges (
            id TEXT PRIMARY KEY,
            source_node_id TEXT NOT NULL,
            target_node_id TEXT NOT NULL,
            relation TEXT NOT NULL,
            confidence TEXT NOT NULL CHECK (confidence IN ('EXTRACTED', 'INFERRED', 'AMBIGUOUS')),
            status TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'pending', 'rejected', 'superseded')),
            evidence TEXT,
            source_ref TEXT,
            scope TEXT,
            weight REAL NOT NULL DEFAULT 1.0,
            origin TEXT NOT NULL CHECK (origin IN ('deterministic', 'semantic')),
            metadata TEXT NOT NULL DEFAULT '{}' CHECK (json_valid(metadata) AND json_type(metadata) = 'object'),
            created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
            updated_at DATETIME DEFAULT CURRENT_TIMESTAMP,
            FOREIGN KEY(source_node_id) REFERENCES graph_nodes(id),
            FOREIGN KEY(target_node_id) REFERENCES graph_nodes(id)
        );

        CREATE INDEX IF NOT EXISTS idx_graph_nodes_kind ON graph_nodes(kind);
        CREATE INDEX IF NOT EXISTS idx_graph_nodes_label ON graph_nodes(label);
        CREATE INDEX IF NOT EXISTS idx_graph_edges_source ON graph_edges(source_node_id);
        CREATE INDEX IF NOT EXISTS idx_graph_edges_target ON graph_edges(target_node_id);
        CREATE INDEX IF NOT EXISTS idx_graph_edges_relation ON graph_edges(relation);
        CREATE INDEX IF NOT EXISTS idx_graph_edges_status ON graph_edges(status);
        CREATE INDEX IF NOT EXISTS idx_graph_edges_confidence ON graph_edges(confidence);
        CREATE INDEX IF NOT EXISTS idx_graph_semantic_edges_status ON graph_semantic_edges(status);
        CREATE INDEX IF NOT EXISTS idx_graph_semantic_edges_source ON graph_semantic_edges(source_ref);
        CREATE INDEX IF NOT EXISTS idx_graph_semantic_edges_target ON graph_semantic_edges(target_ref);
        CREATE INDEX IF NOT EXISTS idx_graph_semantic_edges_relation ON graph_semantic_edges(relation);
        CREATE INDEX IF NOT EXISTS idx_graph_semantic_edges_ambiguity ON graph_semantic_edges(ambiguity_id);
        CREATE INDEX IF NOT EXISTS idx_graph_semantic_revisions_edge ON graph_semantic_edge_revisions(edge_id, version);",
    )
    .context("create graph tables")?;
    conn.execute(
        "INSERT INTO metadata (key, value, updated_at)
         VALUES (?1, ?2, CURRENT_TIMESTAMP)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
        params![GRAPH_SCHEMA_VERSION_KEY, GRAPH_SCHEMA_VERSION.to_string()],
    )?;
    conn.execute(
        "INSERT INTO metadata (key, value, updated_at)
         VALUES (?1, 'true', CURRENT_TIMESTAMP)
         ON CONFLICT(key) DO NOTHING",
        params![GRAPH_DIRTY_KEY],
    )?;
    Ok(())
}

fn migrate_agent_memory_v5(conn: &Connection) -> Result<()> {
    migrate_core_event_tables_v4(conn)?;
    // Some pre-release v4 stores may have been created before every graph-v4
    // compatibility column landed. Add the missing column before recreating
    // indexes that reference it, then run the idempotent table creation.
    if table_exists(conn, "graph_semantic_edges")?
        && !column_exists(conn, "graph_semantic_edges", "ambiguity_id")?
    {
        conn.execute(
            "ALTER TABLE graph_semantic_edges ADD COLUMN ambiguity_id INTEGER REFERENCES ambiguities(id)",
            [],
        )?;
    }
    migrate_graph_v4(conn)?;
    if !column_exists(conn, "graph_semantic_edges", "user_confirmed_at")? {
        conn.execute(
            "ALTER TABLE graph_semantic_edges ADD COLUMN user_confirmed_at DATETIME",
            [],
        )?;
    }

    ensure_store_id(conn)?;
    conn.execute_batch(
        "CREATE TABLE memories_v5 (
            id TEXT PRIMARY KEY,
            type TEXT NOT NULL CHECK (type IN ('user', 'feedback', 'project', 'reference', 'preference', 'workflow')),
            name TEXT NOT NULL,
            description TEXT,
            content TEXT,
            tags TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(tags) AND json_type(tags) = 'array'),
            scope TEXT NOT NULL DEFAULT 'global',
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
            last_accessed_at DATETIME,
            origin TEXT NOT NULL DEFAULT 'direct' CHECK (origin IN ('direct', 'import', 'merge', 'bundle', 'sync', 'migration')),
            origin_ref TEXT,
            user_confirmed_at DATETIME,
            UNIQUE(scope, name)
        );
        INSERT INTO memories_v5
        (id, type, name, description, content, tags, scope, source, confidence, protected,
         created_at, updated_at, expires_at, valid_until, superseded_by, version,
         access_count, last_accessed_at, origin, origin_ref, user_confirmed_at)
        SELECT id, type, name, description, content, tags, COALESCE(scope, 'global'), source,
               confidence, protected, created_at, updated_at, expires_at, valid_until,
               superseded_by, version, access_count, last_accessed_at, 'migration', NULL,
               CASE WHEN source = 'manual' THEN created_at ELSE NULL END
        FROM memories;
        DROP TABLE memories;
        ALTER TABLE memories_v5 RENAME TO memories;
        CREATE INDEX IF NOT EXISTS idx_name_scope ON memories(name, scope);
        CREATE INDEX IF NOT EXISTS idx_type ON memories(type);
        CREATE INDEX IF NOT EXISTS idx_scope ON memories(scope);
        CREATE INDEX IF NOT EXISTS idx_expires ON memories(expires_at);
        CREATE INDEX IF NOT EXISTS idx_source ON memories(source);
        CREATE INDEX IF NOT EXISTS idx_valid_until ON memories(valid_until);
        CREATE INDEX IF NOT EXISTS idx_access ON memories(access_count);
        CREATE INDEX IF NOT EXISTS idx_confidence ON memories(confidence);",
    )
    .context("migrate memories and durable event identities to schema v5")?;

    ensure_event_uid(conn, "ambiguities", "ambiguity", "idx_ambiguities_uid")?;
    ensure_event_uid(conn, "changelog", "change", "idx_changelog_uid")?;
    ensure_event_uid(
        conn,
        "workflow_runs",
        "workflow-run",
        "idx_workflow_runs_uid",
    )?;
    ensure_event_uid(
        conn,
        "graph_semantic_edge_revisions",
        "semantic-revision",
        "idx_graph_semantic_revisions_uid",
    )?;
    set_index_dirty(conn, true)?;
    crate::graph::set_graph_dirty(conn, true)?;
    Ok(())
}

pub fn schema_compatibility_required(conn: &Connection) -> Result<bool> {
    let has_store_id: bool = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM metadata WHERE key = 'store_id' AND value <> '')",
            [],
            |row| row.get(0),
        )
        .unwrap_or(false);
    if !has_store_id
        || !table_exists(conn, "graph_semantic_edges")?
        || !column_exists(conn, "graph_semantic_edges", "ambiguity_id")?
        || !column_exists(conn, "graph_semantic_edges", "user_confirmed_at")?
    {
        return Ok(true);
    }

    for (table, index) in [
        ("ambiguities", "idx_ambiguities_uid"),
        ("changelog", "idx_changelog_uid"),
        ("workflow_runs", "idx_workflow_runs_uid"),
        (
            "graph_semantic_edge_revisions",
            "idx_graph_semantic_revisions_uid",
        ),
    ] {
        if !table_exists(conn, table)? || !column_exists(conn, table, "uid")? {
            return Ok(true);
        }
        let invalid_uid: bool = conn.query_row(
            &format!("SELECT EXISTS(SELECT 1 FROM {table} WHERE uid IS NULL OR uid = '')"),
            [],
            |row| row.get(0),
        )?;
        let unique_index: bool = conn.query_row(
            &format!(
                "SELECT EXISTS(SELECT 1 FROM pragma_index_list('{table}') \
                 WHERE name = ?1 AND \"unique\" = 1)"
            ),
            [index],
            |row| row.get(0),
        )?;
        let triggers: i64 = conn.query_row(
            "SELECT COUNT(*) FROM sqlite_master
             WHERE type = 'trigger' AND name IN (?1, ?2)",
            params![
                format!("enforce_{table}_uid_insert"),
                format!("enforce_{table}_uid_update")
            ],
            |row| row.get(0),
        )?;
        if invalid_uid || !unique_index || triggers != 2 {
            return Ok(true);
        }
    }
    Ok(false)
}

fn repair_v5_compatibility(conn: &Connection) -> Result<()> {
    migrate_metadata_v3(conn)?;
    migrate_core_event_tables_v4(conn)?;
    if table_exists(conn, "graph_semantic_edges")?
        && !column_exists(conn, "graph_semantic_edges", "ambiguity_id")?
    {
        conn.execute(
            "ALTER TABLE graph_semantic_edges ADD COLUMN ambiguity_id INTEGER REFERENCES ambiguities(id)",
            [],
        )?;
    }
    migrate_graph_v4(conn)?;
    if !column_exists(conn, "graph_semantic_edges", "user_confirmed_at")? {
        conn.execute(
            "ALTER TABLE graph_semantic_edges ADD COLUMN user_confirmed_at DATETIME",
            [],
        )?;
    }
    ensure_store_id(conn)?;
    ensure_event_uid(conn, "ambiguities", "ambiguity", "idx_ambiguities_uid")?;
    ensure_event_uid(conn, "changelog", "change", "idx_changelog_uid")?;
    ensure_event_uid(
        conn,
        "workflow_runs",
        "workflow-run",
        "idx_workflow_runs_uid",
    )?;
    ensure_event_uid(
        conn,
        "graph_semantic_edge_revisions",
        "semantic-revision",
        "idx_graph_semantic_revisions_uid",
    )?;
    Ok(())
}

fn ensure_event_uid(conn: &Connection, table: &str, kind: &str, index: &str) -> Result<()> {
    if !column_exists(conn, table, "uid")? {
        conn.execute(&format!("ALTER TABLE {table} ADD COLUMN uid TEXT"), [])?;
    }
    let store_id = store_id(conn)?;
    conn.execute(
        &format!(
            "UPDATE {table} SET uid = ?1 || ':{kind}:legacy:' || id WHERE uid IS NULL OR uid = ''"
        ),
        params![store_id],
    )?;
    let unique_index: bool = conn.query_row(
        &format!(
            "SELECT EXISTS(SELECT 1 FROM pragma_index_list('{table}') \
             WHERE name = ?1 AND \"unique\" = 1)"
        ),
        [index],
        |row| row.get(0),
    )?;
    if !unique_index {
        conn.execute(&format!("DROP INDEX IF EXISTS {index}"), [])?;
    }
    conn.execute(
        &format!("CREATE UNIQUE INDEX IF NOT EXISTS {index} ON {table}(uid)"),
        [],
    )?;
    conn.execute_batch(&format!(
        "CREATE TRIGGER IF NOT EXISTS enforce_{table}_uid_insert
         BEFORE INSERT ON {table}
         WHEN NEW.uid IS NULL OR NEW.uid = ''
         BEGIN SELECT RAISE(ABORT, 'durable event uid is required'); END;
         CREATE TRIGGER IF NOT EXISTS enforce_{table}_uid_update
         BEFORE UPDATE OF uid ON {table}
         WHEN NEW.uid IS NULL OR NEW.uid = ''
         BEGIN SELECT RAISE(ABORT, 'durable event uid is required'); END;"
    ))?;
    Ok(())
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
