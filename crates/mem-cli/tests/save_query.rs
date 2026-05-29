use std::fs;

mod support;

use support::TestRepo;

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
fn query_supports_table_and_compact_formats() {
    let repo = TestRepo::new("query-formats");
    repo.run(&["init"]);
    repo.run(&[
        "save",
        "--name",
        "human_readable",
        "--tags",
        r#"["ui:format"]"#,
        "--content",
        "human readable output content",
        "--force",
    ]);

    let table = repo.run(&["query", "human readable", "--format", "table", "--no-touch"]);
    assert!(table.contains("name"));
    assert!(table.contains("human_readable"));
    assert!(!table.trim_start().starts_with('['));

    let compact = repo.run(&[
        "query",
        "human readable",
        "--format",
        "compact",
        "--no-touch",
    ]);
    assert!(compact.contains("human_readable [reference]"));
    assert!(compact.contains("tags=ui:format"));
}

#[test]
fn query_uses_store_config_default_scope() {
    let repo = TestRepo::new("query-config-scope");
    repo.run(&["init"]);
    fs::write(
        repo.join("config.toml"),
        "[query]\ndefault_scope = \"project:NeoHsu/agent-knowledge\"\n",
    )
    .expect("write config");
    repo.run(&[
        "save",
        "--name",
        "config_global",
        "--scope",
        "global",
        "--content",
        "config scope needle global",
        "--force",
    ]);
    repo.run(&[
        "save",
        "--name",
        "config_project",
        "--scope",
        "project:NeoHsu/agent-knowledge",
        "--content",
        "config scope needle project",
        "--force",
    ]);
    repo.run(&[
        "save",
        "--name",
        "config_other_project",
        "--scope",
        "project:Other/repo",
        "--content",
        "config scope needle other",
        "--force",
    ]);

    let query = repo.run(&["query", "config scope needle", "--no-touch"]);

    assert!(query.contains("config_global"));
    assert!(query.contains("config_project"));
    assert!(!query.contains("config_other_project"));
}

#[test]
fn query_uses_store_config_default_limit() {
    let repo = TestRepo::new("query-config-limit");
    repo.run(&["init"]);
    fs::write(repo.join("config.toml"), "[query]\ndefault_limit = 1\n").expect("write config");
    repo.run(&[
        "save",
        "--name",
        "config_limit_one",
        "--content",
        "config limit first",
        "--force",
    ]);
    repo.run(&[
        "save",
        "--name",
        "config_limit_two",
        "--content",
        "config limit second",
        "--force",
    ]);

    let query = repo.run(&["query"]);
    let rows: serde_json::Value = serde_json::from_str(&query).expect("query json");

    assert_eq!(rows.as_array().expect("query array").len(), 1);
}

#[test]
fn filtered_query_is_not_crowded_out_before_filtering() {
    let repo = TestRepo::new("filtered-query-crowding");
    repo.run(&["init"]);
    for index in 0..25 {
        let name = format!("needle_ref_{index}");
        let content = format!("needle needle needle reference crowd {index}");
        repo.insert_raw_memory(name.as_str(), name.as_str(), content.as_str());
    }
    repo.run(&[
        "save",
        "--type",
        "workflow",
        "--name",
        "needle_workflow",
        "--tags",
        r#"["workflow:test"]"#,
        "--content",
        "schema_version: 1\ngoal: needle workflow\ntriggers:\n  - needle\nsteps:\n  - id: check\n    manual: needle\nstop_conditions:\n  - stop\n",
        "--force",
    ]);
    repo.run(&["reindex"]);

    let query = repo.run(&[
        "query",
        "needle",
        "--type",
        "workflow",
        "--limit",
        "1",
        "--no-touch",
    ]);

    assert!(query.contains("needle_workflow"));
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
