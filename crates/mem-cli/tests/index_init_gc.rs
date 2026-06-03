use std::fs;
use std::process::{Command, Stdio};

use rusqlite::{params, Connection};

mod support;

use support::{mem_bin, temp_path, TestRepo, TestRuntimeStore};

const INDEX_VERSION_MARKER: &str = "index/.mnemark-index-version";

fn mark_index_dirty(repo: &TestRepo) {
    let conn = Connection::open(repo.join("memory.db")).expect("open memory db");
    conn.execute(
        "INSERT INTO metadata (key, value, updated_at)
         VALUES ('index_dirty', 'true', datetime('now'))
         ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
        params![],
    )
    .expect("set index_dirty");
}

fn corrupt_memories_for_reindex(repo: &TestRepo) {
    let conn = Connection::open(repo.join("memory.db")).expect("open memory db");
    conn.execute_batch(
        "DROP TABLE memories;
        CREATE TABLE memories (
            id TEXT PRIMARY KEY,
            type TEXT,
            name TEXT,
            description TEXT,
            content TEXT,
            tags TEXT,
            scope TEXT,
            source TEXT,
            confidence TEXT,
            protected TEXT,
            created_at TEXT,
            updated_at TEXT,
            expires_at TEXT,
            valid_until TEXT,
            superseded_by TEXT,
            version TEXT,
            access_count TEXT,
            last_accessed_at TEXT
        );
        INSERT INTO memories
        (id, type, name, content, tags, scope, source, confidence, protected, created_at, updated_at, version, access_count)
        VALUES ('bad_reindex', 'reference', 'bad_reindex', 'bad reindex content', '[]', 'global', 'manual', 'high', 'not-bool', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z', 'not-an-int', '0');
        PRAGMA user_version = 3;",
    )
    .expect("corrupt memories table");
}

#[test]
fn init_uses_user_config_knowledge_home_when_env_is_absent() {
    let install_dir = temp_path("config-install");
    let run_dir = temp_path("config-run");
    let config_root = temp_path("config-root");
    let knowledge_home = temp_path("config-home");
    fs::create_dir_all(&install_dir).expect("install dir");
    fs::create_dir_all(&run_dir).expect("run dir");
    fs::create_dir_all(config_root.join("mnemark")).expect("config dir");
    let installed_mem = install_dir.join("mem");
    fs::copy(mem_bin(), &installed_mem).expect("copy mem binary");
    fs::write(
        config_root.join("mnemark/config.toml"),
        format!("knowledge_home = \"{}\"\n", knowledge_home.display()),
    )
    .expect("write config");

    let output = Command::new(&installed_mem)
        .current_dir(&run_dir)
        .env("XDG_CONFIG_HOME", &config_root)
        .env_remove("MNEMARK_HOME")
        .arg("init")
        .output()
        .expect("run installed mem init");
    assert!(
        output.status.success(),
        "config init failed\nstdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    assert!(knowledge_home.join("memory.db").exists());
    assert!(knowledge_home.join("index").exists());
    assert!(!run_dir.join("memory.db").exists());

    let shown_output = Command::new(&installed_mem)
        .current_dir(&run_dir)
        .env("XDG_CONFIG_HOME", &config_root)
        .env_remove("MNEMARK_HOME")
        .args(["config", "show"])
        .output()
        .expect("run config show");
    assert!(shown_output.status.success(), "config show failed");
    let shown: serde_json::Value =
        serde_json::from_slice(&shown_output.stdout).expect("config show json");
    assert_eq!(shown["store_source"], "user_config");
    assert_eq!(shown["root"], knowledge_home.display().to_string());

    fs::remove_dir_all(install_dir).ok();
    fs::remove_dir_all(run_dir).ok();
    fs::remove_dir_all(config_root).ok();
    fs::remove_dir_all(knowledge_home).ok();
}

#[test]
fn cli_home_overrides_current_directory_store() {
    let repo = TestRepo::new("cli-home-repo");
    let cli_home = temp_path("cli-home-store");
    let config_root = temp_path("cli-home-config");
    fs::create_dir_all(&config_root).expect("config root");

    let output = Command::new(mem_bin())
        .current_dir(repo.path())
        .env("XDG_CONFIG_HOME", &config_root)
        .env_remove("MNEMARK_HOME")
        .args(["--home", cli_home.to_str().expect("cli home path"), "init"])
        .output()
        .expect("run mem init with cli home");
    assert!(
        output.status.success(),
        "cli home init failed\nstdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    assert!(cli_home.join("memory.db").exists());
    assert!(!repo.join("memory.db").exists());

    let shown_output = Command::new(mem_bin())
        .current_dir(repo.path())
        .env("XDG_CONFIG_HOME", &config_root)
        .env_remove("MNEMARK_HOME")
        .args([
            "--home",
            cli_home.to_str().expect("cli home path"),
            "config",
            "show",
        ])
        .output()
        .expect("run config show with cli home");
    assert!(shown_output.status.success(), "config show failed");
    let shown: serde_json::Value =
        serde_json::from_slice(&shown_output.stdout).expect("config show json");
    assert_eq!(shown["store_source"], "cli");
    assert_eq!(shown["root"], cli_home.display().to_string());

    fs::remove_dir_all(cli_home).ok();
    fs::remove_dir_all(config_root).ok();
}

#[test]
fn query_help_hides_semantic_until_backend_exists() {
    let output = Command::new(mem_bin())
        .args(["query", "--help"])
        .output()
        .expect("run query help");
    assert!(output.status.success(), "query help failed");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(!stdout.contains("--semantic"));
}

#[test]
fn context_without_detect_points_to_next_steps() {
    let repo = TestRepo::new("context-help");

    let output = repo.run_fail(&["context"]);

    assert!(output.contains("mem context --detect"));
    assert!(output.contains("mem context --help"));
}

#[test]
fn config_show_reports_effective_paths_and_defaults() {
    let repo = TestRepo::new("config-show");
    let config_root = temp_path("config-show-root");
    fs::create_dir_all(&config_root).expect("config root");
    fs::write(
        repo.join("config.toml"),
        "[query]\ndefault_scope = \"auto\"\ndefault_limit = 7\n[workflow]\ndefault_scope = \"project:NeoHsu/mnemark\"\ndefault_limit = 3\n",
    )
    .expect("write store config");

    let output = Command::new(mem_bin())
        .current_dir(repo.path())
        .env("XDG_CONFIG_HOME", &config_root)
        .env_remove("MNEMARK_HOME")
        .args(["config", "show"])
        .output()
        .expect("run config show");
    assert!(
        output.status.success(),
        "config show failed\nstdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let shown: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("config show json");
    assert_eq!(shown["root"], repo.path().display().to_string());
    assert_eq!(shown["store_source"], "current_directory");
    assert_eq!(shown["store_config_exists"], true);
    assert_eq!(shown["effective"]["schema"], "embedded");
    assert_eq!(shown["effective"]["query_default_scope"], "auto");
    assert_eq!(shown["effective"]["query_default_limit"], 7);
    assert_eq!(
        shown["effective"]["workflow_default_scope"],
        "project:NeoHsu/mnemark"
    );
    assert_eq!(shown["effective"]["workflow_default_limit"], 3);

    fs::remove_dir_all(config_root).ok();
}

#[test]
fn history_and_stats_support_human_formats() {
    let repo = TestRepo::new("admin-formats");
    repo.run(&["init"]);
    repo.run(&[
        "save",
        "--name",
        "format_target",
        "--type",
        "feedback",
        "--content",
        "format output target",
        "--force",
    ]);
    repo.run(&["query", "format output"]);

    let history_table = repo.run(&["history", "--format", "table"]);
    assert!(history_table.contains("action"));
    assert!(history_table.contains("format_target"));
    assert!(!history_table.trim_start().starts_with('['));

    let history_compact = repo.run(&["history", "--format", "compact"]);
    assert!(history_compact.contains("format_target"));
    assert!(history_compact.contains("source=agent"));

    let stats_table = repo.run(&["stats", "--format", "table"]);
    assert!(stats_table.contains("metric"));
    assert!(stats_table.contains("type:feedback"));

    let stats_compact = repo.run(&["stats", "--format", "compact"]);
    assert!(stats_compact.contains("total_active: 1"));
    assert!(stats_compact.contains("feedback=1"));
}

#[test]
fn init_uses_embedded_schema_when_runtime_root_has_no_schema_file() {
    let store = TestRuntimeStore::new("portable-runtime");

    store.run(&["init"]);

    assert!(store.home().join("memory.db").exists());
    assert!(store.home().join("index").exists());
    assert!(!store.home().join("schema/memory-schema.sql").exists());
    assert!(!store.run_dir().join("memory.db").exists());

    let shown: serde_json::Value =
        serde_json::from_str(&store.run(&["config", "show"])).expect("config json");
    assert_eq!(shown["store_source"], "environment");
    assert_eq!(shown["effective"]["schema"], "embedded");

    let conn = Connection::open(store.home().join("memory.db")).expect("open portable db");
    let version: i64 = conn
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .expect("schema version");
    assert_eq!(version, 3);
}

#[test]
fn query_repairs_stale_index_marker() {
    let repo = TestRepo::new("stale-index");
    repo.run(&["init"]);
    repo.insert_raw_memory("raw_stale", "raw_stale", "stale repair searchable content");
    mark_index_dirty(&repo);

    let query = repo.run(&["query", "searchable"]);
    assert!(query.contains("raw_stale"));
}

#[test]
fn missing_index_version_marker_triggers_rebuild() {
    let repo = TestRepo::new("missing-index-version");
    repo.run(&["init"]);
    repo.insert_raw_memory(
        "raw_missing_marker",
        "raw_missing_marker",
        "missing marker searchable content",
    );
    fs::remove_file(repo.join(INDEX_VERSION_MARKER)).expect("remove index version marker");

    let query = repo.run(&["query", "missing marker searchable"]);

    assert!(query.contains("raw_missing_marker"));
    assert_eq!(
        fs::read_to_string(repo.join(INDEX_VERSION_MARKER)).expect("read marker"),
        "2\n"
    );
}

#[test]
fn old_index_version_marker_triggers_rebuild() {
    let repo = TestRepo::new("old-index-version");
    repo.run(&["init"]);
    repo.insert_raw_memory(
        "raw_old_marker",
        "raw_old_marker",
        "old marker searchable content",
    );
    fs::write(repo.join(INDEX_VERSION_MARKER), "1\n").expect("write old marker");

    let query = repo.run(&["query", "old marker searchable"]);

    assert!(query.contains("raw_old_marker"));
    assert_eq!(
        fs::read_to_string(repo.join(INDEX_VERSION_MARKER)).expect("read marker"),
        "2\n"
    );
}

#[test]
fn invalid_index_version_marker_triggers_rebuild() {
    let repo = TestRepo::new("invalid-index-version");
    repo.run(&["init"]);
    repo.insert_raw_memory(
        "raw_invalid_marker",
        "raw_invalid_marker",
        "invalid marker searchable content",
    );
    fs::write(repo.join(INDEX_VERSION_MARKER), "not-a-version\n").expect("write invalid marker");

    let query = repo.run(&["query", "invalid marker searchable"]);

    assert!(query.contains("raw_invalid_marker"));
    assert_eq!(
        fs::read_to_string(repo.join(INDEX_VERSION_MARKER)).expect("read marker"),
        "2\n"
    );
}

#[test]
fn matching_index_version_marker_does_not_rebuild() {
    let repo = TestRepo::new("matching-index-version");
    repo.run(&["init"]);
    repo.run(&[
        "save",
        "--name",
        "visible_matching_marker",
        "--type",
        "reference",
        "--content",
        "matching marker visible content",
        "--force",
    ]);
    repo.insert_raw_memory(
        "raw_matching_marker",
        "raw_matching_marker",
        "matching marker hidden content",
    );

    let query = repo.run(&["query", "matching marker"]);

    assert!(query.contains("visible_matching_marker"));
    assert!(!query.contains("raw_matching_marker"));
}

#[test]
fn failed_automatic_index_rebuild_returns_actionable_error() {
    let repo = TestRepo::new("failed-index-rebuild");
    repo.run(&["init"]);
    fs::write(repo.join(INDEX_VERSION_MARKER), "1\n").expect("write old marker");
    corrupt_memories_for_reindex(&repo);

    let output = repo.run_fail(&["query", "bad reindex"]);

    assert!(output.contains("index schema version mismatch"));
    assert!(output.contains("automatic rebuild failed"));
    assert!(output.contains("mem reindex"));
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
    mark_index_dirty(&repo);

    let reindexed = repo.run(&["reindex"]);
    assert!(reindexed.contains(r#""status":"reindexed""#));

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
