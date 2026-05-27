use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{params, Connection};
use serde_json::Value;

fn mem_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_mem"))
}

fn temp_repo(name: &str) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("agent-knowledge-{name}-{stamp}"));
    fs::create_dir_all(dir.join("schema")).expect("schema dir");
    fs::write(
        dir.join("schema/memory-schema.sql"),
        include_str!("../schema/memory-schema.sql"),
    )
    .expect("schema");
    dir
}

fn run(repo: &PathBuf, args: &[&str]) -> String {
    let output = Command::new(mem_bin())
        .current_dir(repo)
        .args(args)
        .output()
        .expect("run mem");
    assert!(
        output.status.success(),
        "command failed: {:?}\nstdout={}\nstderr={}",
        args,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("utf8 stdout")
}

fn run_fail(repo: &PathBuf, args: &[&str]) -> String {
    let output = Command::new(mem_bin())
        .current_dir(repo)
        .args(args)
        .output()
        .expect("run mem");
    assert!(
        !output.status.success(),
        "command unexpectedly succeeded: {:?}\nstdout={}\nstderr={}",
        args,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn insert_raw_memory(repo: &Path, id: &str, name: &str, content: &str) {
    let conn = Connection::open(repo.join("memory.db")).expect("open memory db");
    conn.execute(
        "INSERT INTO memories
        (id, type, name, content, tags, scope, source, confidence, protected, created_at, updated_at)
        VALUES (?1, 'reference', ?2, ?3, '[]', 'global', 'manual', 'high', 1, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
        params![id, name, content],
    )
    .expect("insert raw memory");
}

#[test]
fn save_query_and_version_conflict() {
    let repo = temp_repo("save-query");
    run(&repo, &["init"]);
    let saved = run(
        &repo,
        &[
            "save",
            "--type",
            "feedback",
            "--name",
            "no_emoji",
            "--scope",
            "global",
            "--source",
            "manual",
            "--tags",
            r#"["style:no-emoji"]"#,
            "--content",
            "不要使用 emoji",
        ],
    );
    assert!(saved.contains(r#""status":"saved""#));

    let query = run(&repo, &["query", "使用"]);
    assert!(query.contains("no_emoji"));

    let conflict = run(
        &repo,
        &[
            "update",
            "no_emoji",
            "--expected-version",
            "99",
            "--source",
            "manual",
            "--content",
            "不要使用 emoji",
        ],
    );
    assert!(conflict.contains("version_conflict"));

    fs::remove_dir_all(repo).ok();
}

#[test]
fn retro_bundle_contains_repository_state() {
    let repo = temp_repo("retro");
    run(&repo, &["init"]);
    let retro = run(&repo, &["retro", "daily"]);
    assert!(retro.contains("retro_bundle"));
    assert!(retro.contains("platform-provided"));
    fs::remove_dir_all(repo).ok();
}

#[test]
fn query_treats_punctuation_as_literal_text() {
    let repo = temp_repo("literal-query");
    run(&repo, &["init"]);
    run(
        &repo,
        &[
            "save",
            "--name",
            "project_scope",
            "--content",
            "Use project:NeoHsu/agent-knowledge as the portable memory scope",
            "--force",
        ],
    );

    let query = run(&repo, &["query", "project:NeoHsu/agent-knowledge"]);
    assert!(query.contains("project_scope"));

    fs::remove_dir_all(repo).ok();
}

#[test]
fn init_sets_schema_version_and_constraints() {
    let repo = temp_repo("schema-version");
    run(&repo, &["init"]);

    let conn = Connection::open(repo.join("memory.db")).expect("open memory db");
    let version: i64 = conn
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .expect("schema version");
    assert_eq!(version, 2);

    let invalid_tags = conn.execute(
        "INSERT INTO memories
        (id, type, name, content, tags, scope, source, confidence, protected)
        VALUES ('bad_tags', 'reference', 'bad_tags', 'content', 'not-json', 'global', 'manual', 'high', 1)",
        [],
    );
    assert!(invalid_tags.is_err());

    fs::remove_dir_all(repo).ok();
}

#[test]
fn init_migrates_v1_database_and_allows_workflow_type() {
    let repo = temp_repo("schema-migrate-v1");
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

    run(&repo, &["init"]);
    let conn = Connection::open(repo.join("memory.db")).expect("open migrated db");
    let version: i64 = conn
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .expect("schema version");
    assert_eq!(version, 2);
    let legacy_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM memories WHERE name = 'legacy_ref'",
            [],
            |row| row.get(0),
        )
        .expect("legacy count");
    assert_eq!(legacy_count, 1);
    drop(conn);

    let saved = run(
        &repo,
        &[
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

    fs::remove_dir_all(repo).ok();
}

#[test]
fn workflow_save_query_export_and_import() {
    let repo = temp_repo("workflow-roundtrip");
    run(&repo, &["init"]);
    let workflow_file = repo.join("release-workflow.yaml");
    fs::write(
        &workflow_file,
        r#"schema_version: 1
goal: Release safely.
triggers:
  - user asks to release
steps:
  - id: inspect
    run: git status --short
    verify: working tree is understood
  - id: push
    run: git push origin main
    confirm: true
stop_conditions:
  - tests fail
"#,
    )
    .expect("write workflow");

    let saved = run(
        &repo,
        &[
            "save",
            "--type",
            "workflow",
            "--name",
            "release_agent_knowledge",
            "--scope",
            "project:NeoHsu/agent-knowledge",
            "--source",
            "manual",
            "--tags",
            r#"["workflow:release","intent:release","tool:git","risk:high","project:NeoHsu/agent-knowledge"]"#,
            "--content-file",
            workflow_file.to_str().expect("workflow path"),
        ],
    );
    assert!(saved.contains(r#""status":"saved""#));

    let query = run(&repo, &["query", "release", "--type", "workflow"]);
    assert!(query.contains("release_agent_knowledge"));

    let found = run(&repo, &["workflow", "find", "release"]);
    assert!(found.contains("release_agent_knowledge"));

    let shown = run(&repo, &["workflow", "show", "release_agent_knowledge"]);
    assert!(shown.contains(r#""type": "workflow""#));

    let validated = run(&repo, &["workflow", "validate", "release_agent_knowledge"]);
    assert!(validated.contains(r#""status": "valid""#));

    let exported = run(&repo, &["export", "--format", "json"]);
    assert!(exported.contains(r#""type": "workflow""#));

    let imported_repo = temp_repo("workflow-import");
    run(&imported_repo, &["init"]);
    let import_file = imported_repo.join("workflows.json");
    fs::write(&import_file, exported).expect("write workflow import");
    let imported = run(
        &imported_repo,
        &["import", import_file.to_str().expect("import path")],
    );
    assert!(imported.contains(r#""saved": 1"#));
    let imported_query = run(&imported_repo, &["query", "--type", "workflow"]);
    assert!(imported_query.contains("release_agent_knowledge"));

    fs::remove_dir_all(repo).ok();
    fs::remove_dir_all(imported_repo).ok();
}

#[test]
fn workflow_find_is_not_crowded_out_by_non_workflow_matches() {
    let repo = temp_repo("workflow-find-crowding");
    run(&repo, &["init"]);
    for index in 0..30 {
        let name = format!("release_note_{index}");
        let content = format!("release process filler note {index}");
        insert_raw_memory(&repo, name.as_str(), name.as_str(), content.as_str());
    }
    run(&repo, &["reindex"]);
    run(
        &repo,
        &[
            "save",
            "--type",
            "workflow",
            "--name",
            "release_workflow",
            "--tags",
            r#"["workflow:release","intent:release"]"#,
            "--content",
            "schema_version: 1\ngoal: Release safely.\ntriggers:\n  - user asks release\nsteps:\n  - id: inspect\n    check: working tree is understood\nstop_conditions:\n  - unsafe state\n",
        ],
    );

    let found = run(&repo, &["workflow", "find", "release", "--limit", "1"]);
    assert!(found.contains("release_workflow"));

    fs::remove_dir_all(repo).ok();
}

#[test]
fn workflow_find_normalizes_intent_tag_separators() {
    let repo = temp_repo("workflow-find-normalized-intent");
    run(&repo, &["init"]);
    run(
        &repo,
        &[
            "save",
            "--type",
            "workflow",
            "--name",
            "ci_triage_workflow",
            "--tags",
            r#"["workflow:ci-triage","intent:fix-ci"]"#,
            "--content",
            "schema_version: 1\ngoal: Triage CI failures.\ntriggers:\n  - user asks to fix ci\nsteps:\n  - id: inspect\n    check: failed job is identified\nstop_conditions:\n  - CI run is inaccessible\n",
        ],
    );

    let found = run(&repo, &["workflow", "find", "fix ci"]);
    assert!(found.contains("ci_triage_workflow"));

    fs::remove_dir_all(repo).ok();
}

#[test]
fn workflow_validation_rejects_invalid_content_without_bypass() {
    let repo = temp_repo("workflow-validation");
    run(&repo, &["init"]);
    let failed = run_fail(
        &repo,
        &[
            "save",
            "--type",
            "workflow",
            "--name",
            "bad_workflow",
            "--tags",
            r#"["workflow:bad"]"#,
            "--content",
            "goal: missing required fields",
        ],
    );
    assert!(failed.contains("workflow missing required field"));

    let bypassed = run(
        &repo,
        &[
            "save",
            "--type",
            "workflow",
            "--name",
            "bad_workflow",
            "--tags",
            r#"["workflow:bad"]"#,
            "--content",
            "goal: missing required fields",
            "--no-validate-workflow",
        ],
    );
    assert!(bypassed.contains(r#""status":"saved""#));

    fs::remove_dir_all(repo).ok();
}

#[test]
fn merge_invalid_workflow_requires_human_review() {
    let local = temp_repo("merge-workflow-local");
    let theirs = temp_repo("merge-workflow-theirs");
    run(&local, &["init"]);
    run(&theirs, &["init"]);
    run(
        &theirs,
        &[
            "save",
            "--type",
            "workflow",
            "--name",
            "invalid_release_workflow",
            "--tags",
            r#"["workflow:release"]"#,
            "--content",
            "goal: missing required fields",
            "--no-validate-workflow",
        ],
    );

    let theirs_db = theirs.join("memory.db");
    let merged = run(&local, &["merge", theirs_db.to_str().expect("db path")]);
    assert!(merged.contains(r#""imported": 0"#));
    assert!(merged.contains(r#""workflow_review_required": 1"#));

    let exported = run(&local, &["export", "--format", "json"]);
    assert!(!exported.contains("invalid_release_workflow"));

    let ambiguities = run(&local, &["ambiguity", "list", "--pending"]);
    assert!(ambiguities.contains("workflow_validation_failed"));
    assert!(ambiguities.contains("fix_or_reject_before_import"));

    fs::remove_dir_all(local).ok();
    fs::remove_dir_all(theirs).ok();
}

#[test]
fn raw_query_supports_tantivy_field_syntax() {
    let repo = temp_repo("raw-query");
    run(&repo, &["init"]);
    run(
        &repo,
        &[
            "save",
            "--name",
            "raw_target",
            "--content",
            "raw query target content",
            "--force",
        ],
    );
    run(
        &repo,
        &[
            "save",
            "--name",
            "raw_other",
            "--content",
            "different content",
            "--force",
        ],
    );

    let query = run(&repo, &["query", "name:raw_target", "--raw-query"]);
    assert!(query.contains("raw_target"));
    assert!(!query.contains("raw_other"));

    fs::remove_dir_all(repo).ok();
}

#[test]
fn query_no_touch_does_not_increment_access_count() {
    let repo = temp_repo("query-no-touch");
    run(&repo, &["init"]);
    run(
        &repo,
        &[
            "save",
            "--name",
            "quiet_query",
            "--content",
            "quiet query access tracking",
            "--force",
        ],
    );

    run(&repo, &["query", "quiet", "--no-touch"]);
    let untouched = run(&repo, &["export", "--format", "json"]);
    assert!(untouched.contains(r#""access_count": 0"#));

    run(&repo, &["query", "quiet"]);
    let touched = run(&repo, &["export", "--format", "json"]);
    assert!(touched.contains(r#""access_count": 1"#));

    fs::remove_dir_all(repo).ok();
}

#[test]
fn tag_filter_uses_exact_json_membership() {
    let repo = temp_repo("tag-filter");
    run(&repo, &["init"]);
    run(
        &repo,
        &[
            "save",
            "--name",
            "no_emoji",
            "--tags",
            r#"["style:no-emoji"]"#,
            "--content",
            "不要使用 emoji",
            "--force",
        ],
    );
    run(
        &repo,
        &[
            "save",
            "--name",
            "lifestyle",
            "--tags",
            r#"["lifestyle"]"#,
            "--content",
            "Lifestyle reference",
            "--force",
        ],
    );

    let style = run(&repo, &["query", "--tags", "style"]);
    assert!(!style.contains("no_emoji"));
    assert!(!style.contains("lifestyle"));

    let exact = run(&repo, &["query", "--tags", "style:no-emoji"]);
    assert!(exact.contains("no_emoji"));
    assert!(!exact.contains("lifestyle"));

    fs::remove_dir_all(repo).ok();
}

#[test]
fn save_regenerates_colliding_slug_ids() {
    let repo = temp_repo("slug-collision");
    run(&repo, &["init"]);
    let first = run(
        &repo,
        &[
            "save",
            "--name",
            "a b",
            "--content",
            "first unique content",
            "--force",
        ],
    );
    let second = run(
        &repo,
        &[
            "save",
            "--name",
            "a_b",
            "--content",
            "second unique content",
            "--force",
        ],
    );

    assert!(first.contains(r#""id":"a_b""#));
    assert!(second.contains(r#""id":"a_b_2""#));

    fs::remove_dir_all(repo).ok();
}

#[test]
fn merge_conflict_records_incoming_snapshot() {
    let local = temp_repo("merge-local");
    let theirs = temp_repo("merge-theirs");
    run(&local, &["init"]);
    run(&theirs, &["init"]);
    run(
        &local,
        &[
            "save",
            "--name",
            "same_name",
            "--source",
            "manual",
            "--content",
            "local content",
            "--force",
        ],
    );
    run(
        &theirs,
        &[
            "save",
            "--name",
            "same_name",
            "--source",
            "manual",
            "--content",
            "incoming content",
            "--force",
        ],
    );

    let theirs_db = theirs.join("memory.db");
    let merged = run(&local, &["merge", theirs_db.to_str().expect("db path")]);
    assert!(merged.contains(r#""conflicts": 1"#));

    let ambiguities = run(&local, &["ambiguity", "list", "--pending"]);
    assert!(ambiguities.contains("incoming content"));
    assert!(ambiguities.contains("merge_conflict"));
    let rows: Value = serde_json::from_str(&ambiguities).expect("ambiguity json");
    assert!(rows[0]["context"].is_object());
    assert_eq!(
        rows[0]["context"]["incoming"]["content"],
        "incoming content"
    );
    assert!(rows[0]["memory_ids"].is_array());

    fs::remove_dir_all(local).ok();
    fs::remove_dir_all(theirs).ok();
}

#[test]
fn merge_strips_secrets_from_raw_incoming_database() {
    let local = temp_repo("merge-strip-local");
    let theirs = temp_repo("merge-strip-theirs");
    run(&local, &["init"]);
    run(&theirs, &["init"]);
    insert_raw_memory(
        &theirs,
        "raw_secret",
        "raw_secret",
        "Authorization: Bearer abcdefghijklmnop",
    );

    let theirs_db = theirs.join("memory.db");
    let merged = run(&local, &["merge", theirs_db.to_str().expect("db path")]);
    assert!(merged.contains(r#""imported": 1"#));

    let exported = run(&local, &["export", "--format", "json"]);
    assert!(exported.contains("[REDACTED]"));
    assert!(!exported.contains("abcdefghijklmnop"));

    fs::remove_dir_all(local).ok();
    fs::remove_dir_all(theirs).ok();
}

#[test]
fn import_outputs_single_summary() {
    let repo = temp_repo("import-summary");
    run(&repo, &["init"]);
    run(
        &repo,
        &[
            "save",
            "--name",
            "existing_import",
            "--content",
            "alpha baseline content",
            "--force",
        ],
    );
    let import_file = repo.join("import.json");
    fs::write(
        &import_file,
        r#"[
          {"name":"new_import","content":"zebra quartz memory payload","tags":["import:test"]},
          {"name":"existing_import","content":"replacement import content"},
          {"content":"missing name"}
        ]"#,
    )
    .expect("write import file");

    let output = run(
        &repo,
        &["import", import_file.to_str().expect("import path")],
    );
    let summary: Value = serde_json::from_str(&output).expect("import summary json");

    assert_eq!(summary["status"], "import_complete");
    assert_eq!(summary["total"], 3);
    assert_eq!(summary["counts"]["saved"], 1);
    assert_eq!(summary["counts"]["duplicate_found"], 1);
    assert_eq!(summary["counts"]["failed"], 1);
    assert!(summary["results"].is_array());

    fs::remove_dir_all(repo).ok();
}

#[test]
fn gc_records_changelog_entries() {
    let repo = temp_repo("gc-history");
    run(&repo, &["init"]);
    run(
        &repo,
        &[
            "save",
            "--name",
            "old_pref",
            "--source",
            "agent",
            "--content",
            "old content",
            "--force",
        ],
    );
    run(
        &repo,
        &[
            "supersede",
            "old_pref",
            "new_pref",
            "--source",
            "agent",
            "--content",
            "new content",
        ],
    );

    let gc = run(&repo, &["gc", "--days=-1"]);
    assert!(gc.contains(r#""deleted":1"#));

    let history = run(&repo, &["history", "--action", "gc"]);
    assert!(history.contains("old_pref") || history.contains("old_content"));

    fs::remove_dir_all(repo).ok();
}
