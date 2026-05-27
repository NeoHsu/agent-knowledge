use std::fs;

use serde_json::Value;

mod support;

use support::TestRepo;

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
