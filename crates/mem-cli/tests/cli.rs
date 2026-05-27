use std::fs;
use std::process::{Command, Stdio};

use rusqlite::Connection;
use serde_json::Value;

mod support;

use support::{mem_bin, TestRepo};

#[test]
fn save_query_and_version_conflict() {
    let repo = TestRepo::new("save-query");
    repo.run(&["init"]);
    let saved = repo.run(&[
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
    ]);
    assert!(saved.contains(r#""status":"saved""#));

    let query = repo.run(&["query", "使用"]);
    assert!(query.contains("no_emoji"));

    let conflict = repo.run(&[
        "update",
        "no_emoji",
        "--expected-version",
        "99",
        "--source",
        "manual",
        "--content",
        "不要使用 emoji",
    ]);
    assert!(conflict.contains("version_conflict"));
}

#[test]
fn retro_bundle_contains_repository_state() {
    let repo = TestRepo::new("retro");
    repo.run(&["init"]);
    let retro = repo.run(&["retro", "daily"]);
    assert!(retro.contains("retro_bundle"));
    assert!(retro.contains("platform-provided"));
}

#[test]
fn query_treats_punctuation_as_literal_text() {
    let repo = TestRepo::new("literal-query");
    repo.run(&["init"]);
    repo.run(&[
        "save",
        "--name",
        "project_scope",
        "--content",
        "Use project:NeoHsu/agent-knowledge as the portable memory scope",
        "--force",
    ]);

    let query = repo.run(&["query", "project:NeoHsu/agent-knowledge"]);
    assert!(query.contains("project_scope"));
}

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
fn workflow_save_query_export_and_import() {
    let repo = TestRepo::new("workflow-roundtrip");
    repo.run(&["init"]);
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

    let saved = repo.run(&[
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

    let query = repo.run(&["query", "release", "--type", "workflow"]);
    assert!(query.contains("release_agent_knowledge"));

    let found = repo.run(&["workflow", "find", "release"]);
    assert!(found.contains("release_agent_knowledge"));

    let shown = repo.run(&["workflow", "show", "release_agent_knowledge"]);
    assert!(shown.contains(r#""type": "workflow""#));

    let validated = repo.run(&["workflow", "validate", "release_agent_knowledge"]);
    assert!(validated.contains(r#""status": "valid""#));

    let exported = repo.run(&["export", "--format", "json"]);
    assert!(exported.contains(r#""type": "workflow""#));

    let imported_repo = TestRepo::new("workflow-import");
    imported_repo.run(&["init"]);
    let import_file = imported_repo.join("workflows.json");
    fs::write(&import_file, exported).expect("write workflow import");
    let imported = imported_repo.run(&["import", import_file.to_str().expect("import path")]);
    assert!(imported.contains(r#""saved": 1"#));
    let imported_query = imported_repo.run(&["query", "--type", "workflow"]);
    assert!(imported_query.contains("release_agent_knowledge"));
}

#[test]
fn workflow_find_is_not_crowded_out_by_non_workflow_matches() {
    let repo = TestRepo::new("workflow-find-crowding");
    repo.run(&["init"]);
    for index in 0..30 {
        let name = format!("release_note_{index}");
        let content = format!("release process filler note {index}");
        repo.insert_raw_memory(name.as_str(), name.as_str(), content.as_str());
    }
    repo.run(&["reindex"]);
    repo.run(&[
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

    let found = repo.run(&["workflow", "find", "release", "--limit", "1"]);
    assert!(found.contains("release_workflow"));
}

#[test]
fn workflow_find_normalizes_intent_tag_separators() {
    let repo = TestRepo::new("workflow-find-normalized-intent");
    repo.run(&["init"]);
    repo.run(&[
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

    let found = repo.run(&["workflow", "find", "fix ci"]);
    assert!(found.contains("ci_triage_workflow"));
}

#[test]
fn workflow_validation_rejects_invalid_content_without_bypass() {
    let repo = TestRepo::new("workflow-validation");
    repo.run(&["init"]);
    let failed = repo.run_fail(&[
        "save",
        "--type",
        "workflow",
        "--name",
        "bad_workflow",
        "--tags",
        r#"["workflow:bad"]"#,
        "--content",
        "goal: missing required fields",
    ]);
    assert!(failed.contains("workflow missing required field"));

    let bypassed = repo.run(&[
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
    ]);
    assert!(bypassed.contains(r#""status":"saved""#));
}

#[test]
fn workflow_project_scope_requires_matching_project_tag() {
    let repo = TestRepo::new("workflow-project-tag");
    repo.run(&["init"]);

    let failed = repo.run_fail(&[
            "save",
            "--type",
            "workflow",
            "--name",
            "project_workflow_missing_tag",
            "--scope",
            "project:NeoHsu/agent-knowledge",
            "--tags",
            r#"["workflow:project"]"#,
            "--content",
            "schema_version: 1\ngoal: Project workflow.\ntriggers:\n  - project task\nsteps:\n  - id: inspect\n    check: project state is known\nstop_conditions:\n  - missing context\n",
        ],
    );
    assert!(failed.contains("project-scoped workflow requires matching"));

    let saved = repo.run(&[
            "save",
            "--type",
            "workflow",
            "--name",
            "project_workflow",
            "--scope",
            "project:NeoHsu/agent-knowledge",
            "--tags",
            r#"["workflow:project","project:NeoHsu/agent-knowledge"]"#,
            "--content",
            "schema_version: 1\ngoal: Project workflow.\ntriggers:\n  - project task\nsteps:\n  - id: inspect\n    check: project state is known\nstop_conditions:\n  - missing context\n",
        ],
    );
    assert!(saved.contains(r#""status":"saved""#));
}

#[test]
fn merge_invalid_workflow_requires_human_review() {
    let local = TestRepo::new("merge-workflow-local");
    let theirs = TestRepo::new("merge-workflow-theirs");
    local.run(&["init"]);
    theirs.run(&["init"]);
    theirs.run(&[
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
    ]);

    let theirs_db = theirs.join("memory.db");
    let merged = local.run(&["merge", theirs_db.to_str().expect("db path")]);
    assert!(merged.contains(r#""imported": 0"#));
    assert!(merged.contains(r#""workflow_review_required": 1"#));

    let exported = local.run(&["export", "--format", "json"]);
    assert!(!exported.contains("invalid_release_workflow"));

    let ambiguities = local.run(&["ambiguity", "list", "--pending"]);
    assert!(ambiguities.contains("workflow_validation_failed"));
    assert!(ambiguities.contains("fix_or_reject_before_import"));
}

#[test]
fn raw_query_supports_tantivy_field_syntax() {
    let repo = TestRepo::new("raw-query");
    repo.run(&["init"]);
    repo.run(&[
        "save",
        "--name",
        "raw_target",
        "--content",
        "raw query target content",
        "--force",
    ]);
    repo.run(&[
        "save",
        "--name",
        "raw_other",
        "--content",
        "different content",
        "--force",
    ]);

    let query = repo.run(&["query", "name:raw_target", "--raw-query"]);
    assert!(query.contains("raw_target"));
    assert!(!query.contains("raw_other"));
}

#[test]
fn query_no_touch_does_not_increment_access_count() {
    let repo = TestRepo::new("query-no-touch");
    repo.run(&["init"]);
    repo.run(&[
        "save",
        "--name",
        "quiet_query",
        "--content",
        "quiet query access tracking",
        "--force",
    ]);

    repo.run(&["query", "quiet", "--no-touch"]);
    let untouched = repo.run(&["export", "--format", "json"]);
    assert!(untouched.contains(r#""access_count": 0"#));

    repo.run(&["query", "quiet"]);
    let touched = repo.run(&["export", "--format", "json"]);
    assert!(touched.contains(r#""access_count": 1"#));
}

#[test]
fn tag_filter_uses_exact_json_membership() {
    let repo = TestRepo::new("tag-filter");
    repo.run(&["init"]);
    repo.run(&[
        "save",
        "--name",
        "no_emoji",
        "--tags",
        r#"["style:no-emoji"]"#,
        "--content",
        "不要使用 emoji",
        "--force",
    ]);
    repo.run(&[
        "save",
        "--name",
        "lifestyle",
        "--tags",
        r#"["lifestyle"]"#,
        "--content",
        "Lifestyle reference",
        "--force",
    ]);

    let style = repo.run(&["query", "--tags", "style"]);
    assert!(!style.contains("no_emoji"));
    assert!(!style.contains("lifestyle"));

    let exact = repo.run(&["query", "--tags", "style:no-emoji"]);
    assert!(exact.contains("no_emoji"));
    assert!(!exact.contains("lifestyle"));
}

#[test]
fn save_regenerates_colliding_slug_ids() {
    let repo = TestRepo::new("slug-collision");
    repo.run(&["init"]);
    let first = repo.run(&[
        "save",
        "--name",
        "a b",
        "--content",
        "first unique content",
        "--force",
    ]);
    let second = repo.run(&[
        "save",
        "--name",
        "a_b",
        "--content",
        "second unique content",
        "--force",
    ]);

    assert!(first.contains(r#""id":"a_b""#));
    assert!(second.contains(r#""id":"a_b_2""#));
}

#[test]
fn lower_trust_source_cannot_force_overwrite_manual_memory() {
    let repo = TestRepo::new("source-priority");
    repo.run(&["init"]);
    repo.run(&[
        "save",
        "--name",
        "manual_preference",
        "--source",
        "manual",
        "--content",
        "manual value",
        "--force",
    ]);

    let rejected = repo.run(&[
        "save",
        "--name",
        "manual_preference",
        "--source",
        "agent",
        "--content",
        "agent replacement",
        "--force",
    ]);
    assert!(rejected.contains("lower_trust_source_cannot_overwrite"));

    let exported = repo.run(&["export", "--format", "json"]);
    assert!(exported.contains("manual value"));
    assert!(!exported.contains("agent replacement"));
}

#[test]
fn merge_conflict_records_incoming_snapshot() {
    let local = TestRepo::new("merge-local");
    let theirs = TestRepo::new("merge-theirs");
    local.run(&["init"]);
    theirs.run(&["init"]);
    local.run(&[
        "save",
        "--name",
        "same_name",
        "--source",
        "manual",
        "--content",
        "local content",
        "--force",
    ]);
    theirs.run(&[
        "save",
        "--name",
        "same_name",
        "--source",
        "manual",
        "--content",
        "incoming content",
        "--force",
    ]);

    let theirs_db = theirs.join("memory.db");
    let merged = local.run(&["merge", theirs_db.to_str().expect("db path")]);
    assert!(merged.contains(r#""conflicts": 1"#));

    let ambiguities = local.run(&["ambiguity", "list", "--pending"]);
    assert!(ambiguities.contains("incoming content"));
    assert!(ambiguities.contains("merge_conflict"));
    let rows: Value = serde_json::from_str(&ambiguities).expect("ambiguity json");
    assert!(rows[0]["context"].is_object());
    assert_eq!(
        rows[0]["context"]["incoming"]["content"],
        "incoming content"
    );
    assert!(rows[0]["memory_ids"].is_array());
}

#[test]
fn merge_strips_secrets_from_raw_incoming_database() {
    let local = TestRepo::new("merge-strip-local");
    let theirs = TestRepo::new("merge-strip-theirs");
    local.run(&["init"]);
    theirs.run(&["init"]);
    theirs.insert_raw_memory(
        "raw_secret",
        "raw_secret",
        "Authorization: Bearer abcdefghijklmnop",
    );

    let theirs_db = theirs.join("memory.db");
    let merged = local.run(&["merge", theirs_db.to_str().expect("db path")]);
    assert!(merged.contains(r#""imported": 1"#));

    let exported = local.run(&["export", "--format", "json"]);
    assert!(exported.contains("[REDACTED]"));
    assert!(!exported.contains("abcdefghijklmnop"));
}

#[test]
fn import_outputs_single_summary() {
    let repo = TestRepo::new("import-summary");
    repo.run(&["init"]);
    repo.run(&[
        "save",
        "--name",
        "existing_import",
        "--content",
        "alpha baseline content",
        "--force",
    ]);
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

    let output = repo.run(&["import", import_file.to_str().expect("import path")]);
    let summary: Value = serde_json::from_str(&output).expect("import summary json");

    assert_eq!(summary["status"], "import_complete");
    assert_eq!(summary["total"], 3);
    assert_eq!(summary["counts"]["saved"], 1);
    assert_eq!(summary["counts"]["duplicate_found"], 1);
    assert_eq!(summary["counts"]["failed"], 1);
    assert!(summary["results"].is_array());
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
