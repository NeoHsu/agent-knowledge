use rusqlite::Connection;
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
        "--user-confirmed",
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
        "--user-confirmed",
        "--content",
        "不要使用 emoji",
    ]);
    assert!(conflict.contains("version_conflict"));
}

#[test]
fn lifecycle_mutations_increment_versions_for_optimistic_concurrency() {
    let repo = TestRepo::new("lifecycle-versions");
    repo.run(&["init"]);
    let deleted: serde_json::Value = serde_json::from_str(&repo.run(&[
        "save",
        "--name",
        "delete_me",
        "--tags",
        "[\"test:version\"]",
        "--content",
        "delete lifecycle version probe",
        "--force",
    ]))
    .expect("delete probe save json");
    let deleted_id = deleted["id"].as_str().expect("delete probe id");
    repo.run(&["delete", "delete_me", "--expected-version", "1"]);

    repo.run(&[
        "save",
        "--name",
        "old_version",
        "--tags",
        "[\"test:version\"]",
        "--content",
        "supersede lifecycle version probe",
        "--force",
    ]);
    repo.run(&[
        "supersede",
        "old_version",
        "new_version",
        "--expected-version",
        "1",
        "--content",
        "replacement lifecycle version probe",
    ]);

    let conn = Connection::open(repo.join("memory.db")).expect("open memory db");
    let deleted_version: i64 = conn
        .query_row(
            "SELECT version FROM memories WHERE id = ?1",
            [deleted_id],
            |row| row.get(0),
        )
        .expect("deleted version");
    let superseded_version: i64 = conn
        .query_row(
            "SELECT version FROM memories WHERE name = 'old_version'",
            [],
            |row| row.get(0),
        )
        .expect("superseded version");
    assert_eq!(deleted_version, 2);
    assert_eq!(superseded_version, 2);
    drop(conn);

    let conflict = repo.run(&[
        "update",
        &format!("id:{deleted_id}"),
        "--expected-version",
        "1",
        "--content",
        "stale update must not be accepted",
    ]);
    assert!(conflict.contains("version_conflict"));
}

#[test]
fn ambiguity_soft_delete_increments_memory_version() {
    let repo = TestRepo::new("ambiguity-lifecycle-version");
    repo.run(&["init"]);
    let keep: serde_json::Value = serde_json::from_str(&repo.run(&[
        "save",
        "--name",
        "ambiguity_keep",
        "--content",
        "keep this ambiguity candidate",
        "--force",
    ]))
    .expect("keep save json");
    let drop_memory: serde_json::Value = serde_json::from_str(&repo.run(&[
        "save",
        "--name",
        "ambiguity_drop",
        "--content",
        "drop this ambiguity candidate",
        "--force",
    ]))
    .expect("drop save json");
    let memory_ids = serde_json::to_string(&vec![
        keep["id"].as_str().expect("keep id"),
        drop_memory["id"].as_str().expect("drop id"),
    ])
    .expect("memory ids");
    let ambiguity: serde_json::Value = serde_json::from_str(&repo.run(&[
        "ambiguity",
        "add",
        "--query",
        "choose lifecycle candidate",
        "--memory-ids",
        &memory_ids,
    ]))
    .expect("ambiguity json");
    let ambiguity_id = ambiguity["id"].as_i64().expect("ambiguity id").to_string();
    repo.run(&[
        "ambiguity",
        "resolve",
        &ambiguity_id,
        "--keep",
        "ambiguity_keep",
        "--soft-delete-others",
    ]);

    let conn = Connection::open(repo.join("memory.db")).expect("open memory db");
    let version: i64 = conn
        .query_row(
            "SELECT version FROM memories WHERE name = 'ambiguity_drop'",
            [],
            |row| row.get(0),
        )
        .expect("ambiguity soft-delete version");
    assert_eq!(version, 2);
}

#[test]
fn query_rejects_result_limits_above_candidate_safety_bound() {
    let repo = TestRepo::new("query-result-hard-limit");
    repo.run(&["init"]);
    let error = repo.run_fail(&["query", "bounded", "--limit", "10001"]);
    assert!(error.contains("query --limit cannot exceed 10000"));
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
        "Use project:NeoHsu/mnemark as the portable memory scope",
        "--force",
    ]);

    let query = repo.run(&["query", "project:NeoHsu/mnemark"]);
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
fn fuzzy_query_matches_multiple_typo_terms() {
    let repo = TestRepo::new("fuzzy-multi-term");
    repo.run(&["init"]);
    repo.run(&[
        "save",
        "--name",
        "release_runbook",
        "--content",
        "release workflow checklist",
        "--force",
    ]);

    let query = repo.run(&["query", "releaze workflo", "--fuzzy", "--no-touch"]);

    assert!(query.contains("release_runbook"));
}

#[test]
fn chinese_query_uses_the_index_multilingual_tokenizer() {
    let repo = TestRepo::new("query-chinese-tokenizer");
    repo.run(&["init"]);
    repo.run(&[
        "save",
        "--name",
        "deployment_workflow",
        "--content",
        "正式環境部署流程與回滾檢查清單",
        "--force",
    ]);

    let query = repo.run(&["query", "部署流程", "--no-touch"]);

    assert!(query.contains("deployment_workflow"));
}

#[test]
fn fuzzy_query_tokenizes_chinese_and_english_before_matching() {
    let repo = TestRepo::new("fuzzy-chinese-tokenizer");
    repo.run(&["init"]);
    repo.run(&[
        "save",
        "--name",
        "mixed_release_workflow",
        "--content",
        "release 部署流程 checklist",
        "--force",
    ]);

    let query = repo.run(&["query", "releaze 部署流成", "--fuzzy", "--no-touch"]);

    assert!(query.contains("mixed_release_workflow"));
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
        "[query]\ndefault_scope = \"project:NeoHsu/mnemark\"\n",
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
        "project:NeoHsu/mnemark",
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
fn query_relevance_rerank_is_transparent_and_prefers_trusted_sources() {
    let repo = TestRepo::new("query-rerank");
    repo.run(&["init"]);
    repo.run(&[
        "save",
        "--name",
        "trusted_candidate",
        "--source",
        "manual",
        "--user-confirmed",
        "--confidence",
        "high",
        "--content",
        "shared deterministic retrieval payload",
        "--force",
    ]);
    repo.run(&[
        "save",
        "--name",
        "retro_candidate",
        "--source",
        "weekly_retro",
        "--confidence",
        "low",
        "--content",
        "shared deterministic retrieval payload",
        "--force",
    ]);

    let output: serde_json::Value = serde_json::from_str(&repo.run(&[
        "query",
        "shared deterministic retrieval payload",
        "--explain-score",
    ]))
    .expect("explained query json");
    let rows = output.as_array().expect("query rows");
    assert_eq!(rows[0]["name"], "trusted_candidate");
    assert!(rows[0]["retrieval_score"]["lexical"].is_number());
    assert!(rows[0]["retrieval_score"]["source_trust"].is_number());
    assert!(rows[0]["retrieval_score"]["confidence"].is_number());
    assert!(rows[0]["retrieval_score"]["scope_specificity"].is_number());
    assert!(rows[0]["retrieval_score"]["recency"].is_number());
    assert!(
        rows[0]["retrieval_score"]["total"].as_f64() > rows[1]["retrieval_score"]["total"].as_f64()
    );
}

#[test]
fn query_is_no_touch_by_default_and_touch_is_explicit() {
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

    repo.run(&["query", "quiet"]);
    let untouched = repo.run(&["export", "--format", "json"]);
    assert!(untouched.contains(r#""access_count": 0"#));

    repo.run(&["query", "quiet", "--touch"]);
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
        "--user-confirmed",
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
