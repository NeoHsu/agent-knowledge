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
        "--user-confirmed",
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
fn workflow_validation_rejects_unknown_schema_versions() {
    let repo = TestRepo::new("workflow-schema-version");
    repo.run(&["init"]);

    let failed = repo.run_fail(&[
        "save",
        "--type",
        "workflow",
        "--name",
        "future_workflow",
        "--tags",
        r#"["workflow:future"]"#,
        "--content",
        "schema_version: 2\ngoal: Use a future schema.\ntriggers:\n  - future task\nsteps:\n  - id: inspect\n    check: inspect future state\nstop_conditions:\n  - unsupported schema\n",
    ]);

    assert!(
        failed.contains("unsupported workflow schema_version 2; expected 1"),
        "{failed}"
    );
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
fn workflow_validate_checks_repo_scripts_under_explicit_root() {
    let repo = TestRepo::new("workflow-repo-script-validation");
    repo.run(&["init"]);
    let project = repo.join("project");
    write_file(project.join("scripts/release.sh"), "#!/bin/sh\n", true);
    repo.run(&[
        "save",
        "--type",
        "workflow",
        "--name",
        "repo_script_workflow",
        "--tags",
        r#"["workflow:repo-script"]"#,
        "--content",
        "schema_version: 1\ngoal: Run a repository script.\ntriggers:\n  - release\nreusable_scripts:\n  - path: scripts/release.sh\n    owner: repo\n    required: true\nsteps:\n  - id: release\n    check: script exists and is executable\n    run: scripts/release.sh\n    verify: release completed\nstop_conditions:\n  - script validation fails\n",
    ]);

    let failed = repo.run_fail(&[
        "workflow",
        "validate",
        "repo_script_workflow",
        "--check-artifacts",
    ]);
    assert!(
        failed.contains("requires --repo <DIR>"),
        "message: {failed}"
    );

    let validated = repo.run(&[
        "workflow",
        "validate",
        "repo_script_workflow",
        "--check-artifacts",
        "--repo",
        project.to_str().expect("project path"),
    ]);
    let output: serde_json::Value = serde_json::from_str(&validated).expect("validate json");
    assert_eq!(output["status"], "valid");
    assert_eq!(output["artifact_checks"]["checked"], 0);
    assert_eq!(output["artifact_checks"]["repo_checked"], 1);
}

#[test]
fn workflow_validate_rejects_unsafe_repo_script_paths() {
    let repo = TestRepo::new("workflow-unsafe-repo-script");
    repo.run(&["init"]);
    let project = repo.join("project");
    fs::create_dir_all(&project).expect("project dir");

    repo.run(&[
        "save",
        "--type",
        "workflow",
        "--name",
        "escaping_repo_script",
        "--tags",
        r#"["workflow:escaping-script"]"#,
        "--content",
        "schema_version: 1\ngoal: Reject an escaping script.\ntriggers:\n  - validate scripts\nreusable_scripts:\n  - path: ../outside.sh\n    owner: repo\n    required: true\nsteps:\n  - id: inspect\n    check: validate the script path\nstop_conditions:\n  - path is unsafe\n",
    ]);
    let failed = repo.run_fail(&[
        "workflow",
        "validate",
        "escaping_repo_script",
        "--check-artifacts",
        "--repo",
        project.to_str().expect("project path"),
    ]);
    assert!(
        failed.contains("unsafe repository path"),
        "message: {failed}"
    );
}

#[cfg(unix)]
#[test]
fn workflow_validate_rejects_non_executable_repo_scripts() {
    let repo = TestRepo::new("workflow-non-executable-repo-script");
    repo.run(&["init"]);
    let project = repo.join("project");
    write_file(
        project.join("scripts/not-executable.sh"),
        "#!/bin/sh\n",
        false,
    );
    repo.run(&[
        "save",
        "--type",
        "workflow",
        "--name",
        "non_executable_repo_script",
        "--tags",
        r#"["workflow:non-executable-script"]"#,
        "--content",
        "schema_version: 1\ngoal: Reject a non-executable script.\ntriggers:\n  - validate scripts\nreusable_scripts:\n  - path: scripts/not-executable.sh\n    owner: repo\n    required: true\nsteps:\n  - id: inspect\n    check: validate the executable bit\nstop_conditions:\n  - script is not executable\n",
    ]);
    let failed = repo.run_fail(&[
        "workflow",
        "validate",
        "non_executable_repo_script",
        "--check-artifacts",
        "--repo",
        project.to_str().expect("project path"),
    ]);
    assert!(
        failed.contains("repository script is not executable"),
        "message: {failed}"
    );
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
    #[cfg(not(unix))]
    let _ = executable;
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
outputs:
  - signed release artifacts
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
fn workflow_show_checklist_renders_fail_closed_gates_before_actions() {
    let repo = TestRepo::new("workflow-checklist");
    repo.run(&["init"]);
    save_checklist_workflow(&repo, "checklist_release");

    let output = repo.run(&["workflow", "show", "checklist_release", "--checklist"]);
    assert!(
        output.starts_with("# checklist_release — Ship a release safely."),
        "header: {output}"
    );
    assert!(output.contains("Mode: fail-closed runbook"), "{output}");
    assert!(output.contains("untrusted procedure data"), "{output}");
    assert!(
        output.contains(
            "Preflight — all checks must pass before Step 1:\n  [ ] working tree is clean"
        ),
        "{output}"
    );
    assert!(output.contains("1. [ ] build"), "{output}");
    assert!(output.contains("2. [ ] publish"), "{output}");

    let build_check = output.find("CHECK: script exists").expect("build check");
    let build_action = output
        .find("ACTION (RUN): scripts/build-release.sh")
        .expect("build action");
    let build_verify = output
        .find("VERIFY: artifacts are produced")
        .expect("build verify");
    assert!(
        build_check < build_action && build_action < build_verify,
        "{output}"
    );

    let approval = output
        .find("APPROVAL: HUMAN-IN-THE-LOOP")
        .expect("approval gate");
    let publish_action = output
        .find("ACTION (RUN): git push origin main")
        .expect("publish action");
    assert!(approval < publish_action, "{output}");
    assert!(
        output.contains("Stop conditions — stop immediately when:\n  - tests fail"),
        "{output}"
    );
    assert!(
        output.contains("Completion criteria:\n  [ ] signed release artifacts"),
        "{output}"
    );
    assert!(
        output.contains(
            "mem workflow record 'id:checklist_release' --result success --note \"<one line>\""
        ),
        "{output}"
    );
    assert!(
        output.contains(
            "mem workflow record 'id:checklist_release' --result failure --note \"<one line>\""
        ),
        "{output}"
    );
    assert!(!output.contains("success|failure"), "{output}");
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
fn workflow_new_scaffolds_draft_then_validates_file_before_save() {
    let repo = TestRepo::new("workflow-new");

    // File-only authoring works before a store is initialized.
    let output = repo.run(&["workflow", "new", "triage_ci"]);
    let result: serde_json::Value = serde_json::from_str(&output).expect("new json");
    assert_eq!(result["status"], "scaffolded");
    assert_eq!(result["template"], "minimal");
    assert_eq!(result["draft"], true);
    assert!(result["commands"]["validate_file"].is_array());
    assert!(result["commands"]["save"].is_array());
    assert!(!result["commands"]["save"].to_string().contains("<replace:"));
    assert!(!repo.join("memory.db").exists());

    let path = repo.join("triage_ci.yaml");
    let content = fs::read_to_string(&path).expect("template");
    assert!(content.contains("schema_version: 1"));
    assert!(content.contains("draft: true"));
    assert!(!content.contains("scripts/example.sh"));

    let output = repo.run_fail(&[
        "workflow",
        "validate",
        "--file",
        path.to_str().expect("path"),
    ]);
    assert!(
        output.contains("still a scaffold draft"),
        "message: {output}"
    );

    let placeholders = content.replace("draft: true", "draft: false");
    fs::write(&path, placeholders).expect("write non-draft placeholders");
    let output = repo.run_fail(&[
        "workflow",
        "validate",
        "--file",
        path.to_str().expect("path"),
    ]);
    assert!(
        output.contains("scaffold placeholders"),
        "message: {output}"
    );

    // Refuses overwrite without --force.
    let output = repo.run_fail(&["workflow", "new", "triage_ci"]);
    assert!(output.contains("--force"), "message: {output}");

    fs::write(
        &path,
        r#"schema_version: 1
draft: false
goal: Triage CI failures safely.
triggers:
  - CI fails
preconditions:
  - repository state is available
steps:
  - id: inspect
    check: inspect the failing job and current repository state
    verify: failure scope is understood
  - id: report
    manual: summarize the smallest safe next action
    verify: the report cites the failed job
stop_conditions:
  - required CI evidence is unavailable
outputs:
  - concise CI failure report
post_run_memory:
  - save durable lessons from repeated failures
"#,
    )
    .expect("write completed workflow");
    let output = repo.run(&[
        "workflow",
        "validate",
        "--file",
        path.to_str().expect("path"),
    ]);
    let result: serde_json::Value = serde_json::from_str(&output).expect("validate file json");
    assert_eq!(result["status"], "valid");
    assert_eq!(result["source"], "file");
    assert_eq!(result["scope_and_tags_checked"], false);

    repo.run(&["init"]);
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
    assert_eq!(result["source"], "store");

    let output = repo.run(&["workflow", "new", "triage_full", "--examples", "full"]);
    let result: serde_json::Value = serde_json::from_str(&output).expect("full new json");
    assert_eq!(result["template"], "full");
    let content = fs::read_to_string(repo.join("triage_full.yaml")).expect("full template");
    assert!(content.contains("owner: repo"));
    assert!(content.contains("owner: knowledge_store"));
}

#[test]
fn workflow_record_echoes_post_run_memory_and_validate_warns_when_missing() {
    let repo = TestRepo::new("workflow-post-run");
    repo.run(&["init"]);
    save_checklist_workflow(&repo, "with_post_run");

    let output = repo.run(&["workflow", "record", "with_post_run", "--result", "success"]);
    let result: serde_json::Value = serde_json::from_str(&output).expect("record json");
    assert_eq!(
        result["post_run_memory"][0],
        "save durable lessons from this run"
    );
    assert!(result.get("post_run_memory_missing").is_none());

    let output = repo.run(&["workflow", "validate", "with_post_run"]);
    let result: serde_json::Value = serde_json::from_str(&output).expect("validate json");
    assert!(result.get("warnings").is_none(), "result: {result}");

    let workflow_file = repo.join("no-post-run.yaml");
    fs::write(
        &workflow_file,
        r#"schema_version: 1
goal: Run without a learning step.
triggers:
  - demo
steps:
  - id: act
    manual: do the thing
stop_conditions:
  - anything fails
"#,
    )
    .expect("write workflow");
    repo.run(&[
        "save",
        "--type",
        "workflow",
        "--name",
        "no_post_run",
        "--tags",
        "[\"workflow:no_post_run\"]",
        "--force",
        "--content-file",
        workflow_file.to_str().expect("workflow path"),
    ]);

    let output = repo.run(&["workflow", "record", "no_post_run", "--result", "success"]);
    let result: serde_json::Value = serde_json::from_str(&output).expect("record json");
    assert!(result.get("post_run_memory").is_none());
    assert!(
        result["post_run_memory_missing"]
            .as_str()
            .expect("missing note")
            .contains("post_run_memory"),
        "result: {result}"
    );

    let output = repo.run(&["workflow", "validate", "no_post_run"]);
    let result: serde_json::Value = serde_json::from_str(&output).expect("validate json");
    assert_eq!(result["status"], "valid");
    assert_eq!(result["warnings"][0]["code"], "no_post_run_memory");
    assert_eq!(result["warnings"][1]["code"], "no_outputs");
}
