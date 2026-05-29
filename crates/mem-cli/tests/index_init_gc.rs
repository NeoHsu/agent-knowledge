use std::fs;
use std::process::{Command, Stdio};

use rusqlite::{params, Connection};

mod support;

use support::{mem_bin, temp_path, TestRepo, TestRuntimeStore};

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

#[test]
fn init_uses_user_config_knowledge_home_when_env_is_absent() {
    let install_dir = temp_path("config-install");
    let run_dir = temp_path("config-run");
    let config_root = temp_path("config-root");
    let knowledge_home = temp_path("config-home");
    fs::create_dir_all(&install_dir).expect("install dir");
    fs::create_dir_all(&run_dir).expect("run dir");
    fs::create_dir_all(config_root.join("agent-knowledge")).expect("config dir");
    let installed_mem = install_dir.join("mem");
    fs::copy(mem_bin(), &installed_mem).expect("copy mem binary");
    fs::write(
        config_root.join("agent-knowledge/config.toml"),
        format!("knowledge_home = \"{}\"\n", knowledge_home.display()),
    )
    .expect("write config");

    let output = Command::new(&installed_mem)
        .current_dir(&run_dir)
        .env("XDG_CONFIG_HOME", &config_root)
        .env_remove("AGENT_KNOWLEDGE_HOME")
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
        .env_remove("AGENT_KNOWLEDGE_HOME")
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
        .env_remove("AGENT_KNOWLEDGE_HOME")
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
        .env_remove("AGENT_KNOWLEDGE_HOME")
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
fn config_show_reports_effective_paths_and_defaults() {
    let repo = TestRepo::new("config-show");
    let config_root = temp_path("config-show-root");
    fs::create_dir_all(&config_root).expect("config root");
    fs::write(
        repo.join("config.toml"),
        "[query]\ndefault_scope = \"auto\"\ndefault_limit = 7\n[workflow]\ndefault_scope = \"project:NeoHsu/agent-knowledge\"\ndefault_limit = 3\n",
    )
    .expect("write store config");

    let output = Command::new(mem_bin())
        .current_dir(repo.path())
        .env("XDG_CONFIG_HOME", &config_root)
        .env_remove("AGENT_KNOWLEDGE_HOME")
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
        "project:NeoHsu/agent-knowledge"
    );
    assert_eq!(shown["effective"]["workflow_default_limit"], 3);

    fs::remove_dir_all(config_root).ok();
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
