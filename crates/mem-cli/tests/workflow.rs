use std::fs;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

mod support;

use support::TestRepo;

const HELLO_SHA256: &str =
    "sha256:5891b5b522d5df086d0ff0b110fbd9d21bb4fc7163af34d08286a2e846f6be03";

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
        "release_mnemark",
        "--scope",
        "project:NeoHsu/mnemark",
        "--source",
        "manual",
        "--tags",
        r#"["workflow:release","intent:release","tool:git","risk:high","project:NeoHsu/mnemark"]"#,
        "--content-file",
        workflow_file.to_str().expect("workflow path"),
    ]);
    assert!(saved.contains(r#""status":"saved""#));

    let query = repo.run(&["query", "release", "--type", "workflow"]);
    assert!(query.contains("release_mnemark"));

    let found = repo.run(&["workflow", "find", "release"]);
    assert!(found.contains("release_mnemark"));

    let shown = repo.run(&["workflow", "show", "release_mnemark"]);
    assert!(shown.contains(r#""type": "workflow""#));

    let validated = repo.run(&["workflow", "validate", "release_mnemark"]);
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
    assert!(imported_query.contains("release_mnemark"));
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
fn workflow_find_uses_store_config_defaults() {
    let repo = TestRepo::new("workflow-config-defaults");
    repo.run(&["init"]);
    fs::write(
        repo.join("config.toml"),
        "[workflow]\ndefault_scope = \"project:NeoHsu/mnemark\"\ndefault_limit = 1\n",
    )
    .expect("write config");
    let global_content = "schema_version: 1\ngoal: Release global workflow safely.\ntriggers:\n  - release\nsteps:\n  - id: inspect\n    check: global state is known\nstop_conditions:\n  - unsafe state\n";
    let project_content = "schema_version: 1\ngoal: Release project workflow safely.\ntriggers:\n  - release\nsteps:\n  - id: inspect\n    check: project state is known\nstop_conditions:\n  - unsafe state\n";
    let other_content = "schema_version: 1\ngoal: Release other project workflow safely.\ntriggers:\n  - release\nsteps:\n  - id: inspect\n    check: other project state is known\nstop_conditions:\n  - unsafe state\n";
    repo.run(&[
        "save",
        "--type",
        "workflow",
        "--name",
        "release_global_workflow",
        "--scope",
        "global",
        "--tags",
        r#"["workflow:release","intent:release"]"#,
        "--content",
        global_content,
        "--force",
    ]);
    repo.run(&[
        "save",
        "--type",
        "workflow",
        "--name",
        "release_project_workflow",
        "--scope",
        "project:NeoHsu/mnemark",
        "--tags",
        r#"["workflow:release","intent:release","project:NeoHsu/mnemark"]"#,
        "--content",
        project_content,
        "--force",
    ]);
    repo.run(&[
        "save",
        "--type",
        "workflow",
        "--name",
        "release_other_project_workflow",
        "--scope",
        "project:Other/repo",
        "--tags",
        r#"["workflow:release","intent:release","project:Other/repo"]"#,
        "--content",
        other_content,
        "--force",
    ]);

    let found = repo.run(&["workflow", "find", "release"]);

    assert!(found.contains("release_project_workflow"));
    assert!(!found.contains("release_global_workflow"));
    assert!(!found.contains("release_other_project_workflow"));
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
fn workflow_validate_can_check_knowledge_store_artifacts() {
    let repo = TestRepo::new("workflow-artifact-validation");
    repo.run(&["init"]);
    write_file(repo.join("artifacts/scripts/ci-triage.sh"), "hello\n", true);
    repo.run(&[
        "artifact",
        "add",
        "artifacts/scripts/ci-triage.sh",
        "--kind",
        "script",
        "--scope",
        "global",
        "--executable",
    ]);
    repo.run(&[
        "save",
        "--type",
        "workflow",
        "--name",
        "ci_triage_workflow",
        "--tags",
        r#"["workflow:ci-triage","intent:ci-triage"]"#,
        "--content",
        &format!(
            "schema_version: 1\ngoal: Triage CI.\ntriggers:\n  - ci fails\nreusable_scripts:\n  - path: artifacts/scripts/ci-triage.sh\n    owner: knowledge_store\n    required: true\n    checksum: {HELLO_SHA256}\nsteps:\n  - id: collect\n    run: artifacts/scripts/ci-triage.sh\n    check: artifact exists\nstop_conditions:\n  - missing artifact\n"
        ),
    ]);

    let validated = repo.run(&[
        "workflow",
        "validate",
        "ci_triage_workflow",
        "--check-artifacts",
    ]);
    let output: serde_json::Value = serde_json::from_str(&validated).expect("validate json");

    assert_eq!(output["status"], "valid");
    assert_eq!(output["artifact_checks"]["checked"], 1);
}

#[test]
fn workflow_validate_reports_missing_artifact_manifest() {
    let repo = TestRepo::new("workflow-artifact-missing-manifest");
    repo.run(&["init"]);
    repo.run(&[
        "save",
        "--type",
        "workflow",
        "--name",
        "missing_artifact_workflow",
        "--tags",
        r#"["workflow:missing-artifact"]"#,
        "--content",
        "schema_version: 1\ngoal: Missing artifact.\ntriggers:\n  - check artifact\nreusable_scripts:\n  - path: artifacts/scripts/missing.sh\n    owner: knowledge_store\n    required: true\nsteps:\n  - id: collect\n    check: artifact exists\nstop_conditions:\n  - missing artifact\n",
    ]);

    let failed = repo.run_fail(&[
        "workflow",
        "validate",
        "missing_artifact_workflow",
        "--check-artifacts",
    ]);

    assert!(failed.contains("manifest.toml is missing"));
}

#[test]
fn workflow_validate_rejects_step_artifact_run_without_reusable_script_entry() {
    let repo = TestRepo::new("workflow-artifact-undocumented-step");
    repo.run(&["init"]);
    repo.run(&[
        "save",
        "--type",
        "workflow",
        "--name",
        "undocumented_artifact_workflow",
        "--tags",
        r#"["workflow:undocumented-artifact"]"#,
        "--content",
        "schema_version: 1\ngoal: Missing reusable script declaration.\ntriggers:\n  - check artifact\nsteps:\n  - id: collect\n    run: artifacts/scripts/missing.sh\nstop_conditions:\n  - missing artifact\n",
    ]);

    let failed = repo.run_fail(&[
        "workflow",
        "validate",
        "undocumented_artifact_workflow",
        "--check-artifacts",
    ]);

    assert!(failed.contains("reusable_scripts entry is missing"));
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
            "project:NeoHsu/mnemark",
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
            "project:NeoHsu/mnemark",
            "--tags",
            r#"["workflow:project","project:NeoHsu/mnemark"]"#,
            "--content",
            "schema_version: 1\ngoal: Project workflow.\ntriggers:\n  - project task\nsteps:\n  - id: inspect\n    check: project state is known\nstop_conditions:\n  - missing context\n",
        ],
    );
    assert!(saved.contains(r#""status":"saved""#));
}

fn write_file(path: std::path::PathBuf, content: &str, executable: bool) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("artifact dir");
    }
    fs::write(&path, content).expect("write artifact");
    #[cfg(unix)]
    if executable {
        let mut permissions = fs::metadata(&path).expect("metadata").permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&path, permissions).expect("chmod artifact");
    }
}

fn save_checklist_workflow(repo: &TestRepo, name: &str) {
    let workflow_file = repo.join(format!("{name}.yaml"));
    fs::write(
        &workflow_file,
        r#"schema_version: 1
goal: Ship a release safely.
triggers:
  - release
preconditions:
  - working tree is clean
steps:
  - id: build
    run: scripts/build-release.sh
    check: script exists and is executable
    verify: artifacts are produced
  - id: publish
    run: git push origin main
    confirm: true
    verify: remote is updated
stop_conditions:
  - tests fail
post_run_memory:
  - save durable lessons from this run
"#,
    )
    .expect("write workflow");
    repo.run(&[
        "save",
        "--type",
        "workflow",
        "--name",
        name,
        "--tags",
        &format!("[\"workflow:{name}\",\"intent:release\"]"),
        "--content-file",
        workflow_file.to_str().expect("workflow path"),
    ]);
}

#[test]
fn workflow_show_checklist_renders_ordered_steps() {
    let repo = TestRepo::new("workflow-checklist");
    repo.run(&["init"]);
    save_checklist_workflow(&repo, "checklist_release");

    let output = repo.run(&["workflow", "show", "checklist_release", "--checklist"]);
    assert!(
        output.starts_with("# checklist_release — Ship a release safely."),
        "header: {output}"
    );
    assert!(
        output.contains("Preconditions:\n  [ ] working tree is clean"),
        "{output}"
    );
    assert!(output.contains("1. [ ] build"), "{output}");
    assert!(
        output.contains("2. [ ] publish  !! CONFIRM WITH USER BEFORE RUNNING"),
        "{output}"
    );
    assert!(output.contains("run: scripts/build-release.sh"), "{output}");
    assert!(
        output.contains("Stop immediately when:\n  - tests fail"),
        "{output}"
    );
    assert!(
        output.contains("mem workflow record checklist_release --result success|failure"),
        "{output}"
    );
    assert!(
        output.contains("[ ] save durable lessons from this run"),
        "{output}"
    );
}

#[test]
fn workflow_record_tracks_runs_and_feeds_retro() {
    let repo = TestRepo::new("workflow-record");
    repo.run(&["init"]);
    save_checklist_workflow(&repo, "recorded_release");

    let output = repo.run(&[
        "workflow",
        "record",
        "recorded_release",
        "--result",
        "success",
        "--note",
        "clean run",
    ]);
    let result: serde_json::Value = serde_json::from_str(&output).expect("record json");
    assert_eq!(result["status"], "recorded");
    assert_eq!(result["runs_total"], 1);
    assert_eq!(result["failures_total"], 0);
    assert!(result.get("hint").is_none());

    let output = repo.run(&[
        "workflow",
        "record",
        "recorded_release",
        "--result",
        "failure",
        "--note",
        "push rejected",
    ]);
    let result: serde_json::Value = serde_json::from_str(&output).expect("record json");
    assert_eq!(result["runs_total"], 2);
    assert_eq!(result["failures_total"], 1);
    assert!(result["hint"].as_str().expect("hint").contains("mem save"));

    let output = repo.run(&["retro", "weekly"]);
    let bundle: serde_json::Value = serde_json::from_str(&output).expect("retro json");
    let runs = bundle["workflow_runs"].as_array().expect("workflow_runs");
    assert_eq!(runs.len(), 1, "bundle: {bundle}");
    assert_eq!(runs[0]["name"], "recorded_release");
    assert_eq!(runs[0]["runs"], 2);
    assert_eq!(runs[0]["failures"], 1);
    assert_eq!(runs[0]["last_result"], "failure");
}

#[test]
fn workflow_record_rejects_non_workflow_memory() {
    let repo = TestRepo::new("workflow-record-reject");
    repo.run(&["init"]);
    repo.run(&[
        "save",
        "--type",
        "feedback",
        "--name",
        "not_a_workflow",
        "--tags",
        "[\"domain:test\"]",
        "--content",
        "plain feedback memory",
    ]);
    let output = repo.run_fail(&[
        "workflow",
        "record",
        "not_a_workflow",
        "--result",
        "success",
    ]);
    assert!(output.contains("not a workflow"), "message: {output}");
}

#[test]
fn workflow_new_scaffolds_valid_template() {
    let repo = TestRepo::new("workflow-new");
    repo.run(&["init"]);

    let output = repo.run(&["workflow", "new", "triage_ci"]);
    let result: serde_json::Value = serde_json::from_str(&output).expect("new json");
    assert_eq!(result["status"], "scaffolded");
    let path = repo.join("triage_ci.yaml");
    assert!(path.exists());
    let content = fs::read_to_string(&path).expect("template");
    assert!(content.contains("schema_version: 1"));
    assert!(content.contains("must be"), "quoting note: {content}");

    // Refuses overwrite without --force.
    let output = repo.run_fail(&["workflow", "new", "triage_ci"]);
    assert!(output.contains("--force"), "message: {output}");

    // The scaffold saves and validates as-is (template must stay valid).
    repo.run(&[
        "save",
        "--type",
        "workflow",
        "--name",
        "triage_ci",
        "--tags",
        "[\"workflow:triage_ci\"]",
        "--content-file",
        path.to_str().expect("path"),
    ]);
    let output = repo.run(&["workflow", "validate", "triage_ci"]);
    let result: serde_json::Value = serde_json::from_str(&output).expect("validate json");
    assert_eq!(result["status"], "valid");
}
