use std::fs;
use std::process::{Command, Stdio};

use rusqlite::Connection;

mod support;

use support::{mem_bin, TestRepo};

#[test]
fn query_repairs_stale_index_marker() {
    let repo = TestRepo::new("stale-index");
    repo.run(&["init"]);
    repo.insert_raw_memory("raw_stale", "raw_stale", "stale repair searchable content");
    fs::write(repo.join("index/.stale"), r#"{"status":"stale"}"#).expect("stale marker");

    let query = repo.run(&["query", "searchable"]);
    assert!(query.contains("raw_stale"));
    assert!(!repo.join("index/.stale").exists());
}

#[test]
fn reindex_clears_stale_marker_and_indexes_raw_rows() {
    let repo = TestRepo::new("reindex-stale");
    repo.run(&["init"]);
    repo.insert_raw_memory(
        "raw_reindex",
        "raw_reindex",
        "manual reindex searchable content",
    );
    fs::write(repo.join("index/.stale"), r#"{"status":"stale"}"#).expect("stale marker");

    let reindexed = repo.run(&["reindex"]);
    assert!(reindexed.contains(r#""status":"reindexed""#));
    assert!(!repo.join("index/.stale").exists());

    let query = repo.run(&["query", "manual reindex"]);
    assert!(query.contains("raw_reindex"));
}

#[test]
fn concurrent_saves_are_serialized_by_lock() {
    let repo = TestRepo::new("concurrent-save");
    repo.run(&["init"]);

    let mut children = Vec::new();
    for index in 0..4 {
        let name = format!("concurrent_{index}");
        let content = format!("concurrent unique payload {index}");
        let child = Command::new(mem_bin())
            .current_dir(repo.path())
            .args([
                "save",
                "--name",
                name.as_str(),
                "--content",
                content.as_str(),
                "--force",
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn concurrent save");
        children.push((name, child));
    }

    for (name, child) in children {
        let output = child.wait_with_output().expect("wait concurrent save");
        assert!(
            output.status.success(),
            "save {name} failed\nstdout={}\nstderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let exported = repo.run(&["export", "--format", "json"]);
    for index in 0..4 {
        assert!(exported.contains(&format!("concurrent_{index}")));
    }
}

#[test]
fn init_sets_schema_version_and_constraints() {
    let repo = TestRepo::new("schema-version");
    repo.run(&["init"]);

    let conn = Connection::open(repo.join("memory.db")).expect("open memory db");
    let version: i64 = conn
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .expect("schema version");
    assert_eq!(version, 3);
    let metadata_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'metadata'",
            [],
            |row| row.get(0),
        )
        .expect("metadata table count");
    assert_eq!(metadata_count, 1);

    let invalid_tags = conn.execute(
        "INSERT INTO memories
        (id, type, name, content, tags, scope, source, confidence, protected)
        VALUES ('bad_tags', 'reference', 'bad_tags', 'content', 'not-json', 'global', 'manual', 'high', 1)",
        [],
    );
    assert!(invalid_tags.is_err());
}

#[test]
fn init_migrates_v1_database_and_allows_workflow_type() {
    let repo = TestRepo::new("schema-migrate-v1");
    let conn = Connection::open(repo.join("memory.db")).expect("open memory db");
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
        VALUES ('legacy_ref', 'reference', 'legacy_ref', 'legacy content', '[]', 'global', 'manual', 'high', 1, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z');
        PRAGMA user_version = 1;",
    )
    .expect("create v1 db");
    drop(conn);

    repo.run(&["init"]);
    let conn = Connection::open(repo.join("memory.db")).expect("open migrated db");
    let version: i64 = conn
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .expect("schema version");
    assert_eq!(version, 3);
    let legacy_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM memories WHERE name = 'legacy_ref'",
            [],
            |row| row.get(0),
        )
        .expect("legacy count");
    assert_eq!(legacy_count, 1);
    drop(conn);

    let saved = repo.run(&[
            "save",
            "--type",
            "workflow",
            "--name",
            "post_migration_workflow",
            "--tags",
            r#"["workflow:migration"]"#,
            "--content",
            "schema_version: 1\ngoal: Verify migration.\ntriggers:\n  - migration check\nsteps:\n  - id: check\n    manual: inspect migrated database\nstop_conditions:\n  - migration fails\n",
        ],
    );
    assert!(saved.contains(r#""status":"saved""#));
}

#[test]
fn gc_records_changelog_entries() {
    let repo = TestRepo::new("gc-history");
    repo.run(&["init"]);
    repo.run(&[
        "save",
        "--name",
        "old_pref",
        "--source",
        "agent",
        "--content",
        "old content",
        "--force",
    ]);
    repo.run(&[
        "supersede",
        "old_pref",
        "new_pref",
        "--source",
        "agent",
        "--content",
        "new content",
    ]);

    let gc = repo.run(&["gc", "--days=-1"]);
    assert!(gc.contains(r#""deleted":1"#));

    let history = repo.run(&["history", "--action", "gc"]);
    assert!(history.contains("old_pref") || history.contains("old_content"));
}
