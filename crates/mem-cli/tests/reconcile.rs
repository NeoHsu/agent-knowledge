use std::fs;

mod support;

use support::TestRepo;

fn save(repo: &TestRepo, name: &str, content: &str, extra: &[&str]) {
    let mut args = vec![
        "save",
        "--type",
        "project",
        "--name",
        name,
        "--tags",
        "[\"project:example/app\"]",
        "--content",
        content,
    ];
    args.extend_from_slice(extra);
    repo.run(&args);
}

fn reconcile(repo: &TestRepo, extra: &[&str]) -> serde_json::Value {
    let mut args = vec!["reconcile"];
    args.extend_from_slice(extra);
    let output = repo.run(&args);
    serde_json::from_str(&output).expect("reconcile json")
}

fn result_for<'a>(report: &'a serde_json::Value, name: &str) -> &'a serde_json::Value {
    report["results"]
        .as_array()
        .expect("results array")
        .iter()
        .find(|entry| entry["name"] == name)
        .unwrap_or_else(|| panic!("no result for {name}: {report}"))
}

fn claim_status(entry: &serde_json::Value, claim: &str) -> String {
    entry["claims"]
        .as_array()
        .expect("claims array")
        .iter()
        .find(|value| value["claim"] == claim)
        .unwrap_or_else(|| panic!("no claim {claim}: {entry}"))["status"]
        .as_str()
        .expect("status string")
        .to_string()
}

#[test]
fn reconcile_flags_missing_paths_and_confirms_present_ones() {
    let repo = TestRepo::new("reconcile-paths");
    repo.run(&["init"]);
    save(
        &repo,
        "paths_memory",
        "Action: 檢查 `schema/memory-schema.sql` 與 `docs/gone.md`。",
        &[],
    );

    let report = reconcile(&repo, &[]);
    assert_eq!(report["status"], "reconciled");
    assert_eq!(report["memories_checked"], 1);
    assert_eq!(report["memories_flagged"], 1);
    let entry = result_for(&report, "paths_memory");
    assert_eq!(entry["flagged"], true);
    assert_eq!(claim_status(entry, "schema/memory-schema.sql"), "ok");
    assert_eq!(claim_status(entry, "docs/gone.md"), "missing");
}

#[test]
fn reconcile_checks_commands_on_path() {
    let repo = TestRepo::new("reconcile-commands");
    repo.run(&["init"]);
    save(
        &repo,
        "command_memory",
        "Action: 先執行 `git status`，再跑 `definitely-not-a-real-binary-9f2 --help`。",
        &[],
    );

    let report = reconcile(&repo, &[]);
    let entry = result_for(&report, "command_memory");
    assert_eq!(claim_status(entry, "git"), "ok");
    assert_eq!(
        claim_status(entry, "definitely-not-a-real-binary-9f2"),
        "missing"
    );
}

#[test]
fn reconcile_resolves_placeholder_segments() {
    let repo = TestRepo::new("reconcile-placeholder");
    repo.run(&["init"]);
    fs::create_dir_all(repo.join("experts/websocket")).expect("expert dir");
    fs::write(repo.join("experts/websocket/plan.md"), "plan").expect("plan file");
    save(
        &repo,
        "placeholder_memory",
        "Action: 讀 `experts/<domain>/plan.md`，缺的話看 `experts/<domain>/build.md`。",
        &[],
    );

    let report = reconcile(&repo, &[]);
    let entry = result_for(&report, "placeholder_memory");
    assert_eq!(claim_status(entry, "experts/<domain>/plan.md"), "ok");
    assert_eq!(claim_status(entry, "experts/<domain>/build.md"), "missing");
}

#[test]
fn reconcile_reports_unverifiable_spans_without_flagging() {
    let repo = TestRepo::new("reconcile-unverifiable");
    repo.run(&["init"]);
    save(
        &repo,
        "unverifiable_memory",
        "Trigger: 回傳 `duplicate_found` 時。Action: 檢查 `schema/memory-schema.sql`。",
        &[],
    );

    let report = reconcile(&repo, &[]);
    assert_eq!(report["memories_flagged"], 0);
    let entry = result_for(&report, "unverifiable_memory");
    assert_eq!(entry["flagged"], false);
    assert_eq!(entry["unverifiable"][0], "duplicate_found");
}

#[test]
fn reconcile_skips_workflow_memories_unless_type_requested() {
    let repo = TestRepo::new("reconcile-workflow");
    repo.run(&["init"]);
    save(
        &repo,
        "reference_memory",
        "Action: 檢查 `schema/memory-schema.sql`。",
        &[],
    );
    repo.run(&[
        "save",
        "--type",
        "workflow",
        "--name",
        "workflow_memory",
        "--tags",
        "[\"workflow:demo\"]",
        "--no-validate-workflow",
        "--content",
        "goal: run `scripts/does-not-exist.sh`",
    ]);

    let report = reconcile(&repo, &[]);
    assert_eq!(report["memories_checked"], 1);

    let workflow_report = reconcile(&repo, &["--type", "workflow"]);
    assert_eq!(workflow_report["memories_checked"], 1);
    let entry = result_for(&workflow_report, "workflow_memory");
    assert_eq!(claim_status(entry, "scripts/does-not-exist.sh"), "missing");
}

#[test]
fn reconcile_explicit_scope_checks_only_that_scope() {
    let repo = TestRepo::new("reconcile-scope");
    repo.run(&["init"]);
    save(
        &repo,
        "project_memory",
        "Action: 檢查 `docs/gone.md`。",
        &["--scope", "project:example/app"],
    );
    save(
        &repo,
        "global_memory",
        "Action: 檢查 `docs/also-gone.md`。",
        &["--scope", "global"],
    );

    let report = reconcile(&repo, &["--scope", "project:example/app"]);
    assert_eq!(report["memories_checked"], 1);
    assert_eq!(report["scopes"][0], "project:example/app");
    result_for(&report, "project_memory");
}

#[test]
fn reconcile_resolves_relative_paths_against_repo_flag() {
    let repo = TestRepo::new("reconcile-repo-flag");
    repo.run(&["init"]);
    let other = support::temp_path("reconcile-other-repo");
    fs::create_dir_all(other.join("src")).expect("other repo");
    fs::write(other.join("src/lib.rs"), "// lib").expect("lib file");
    save(
        &repo,
        "other_repo_memory",
        "Action: 檢查 `src/lib.rs`。",
        &[],
    );

    let other_str = other.to_string_lossy().to_string();
    let report = reconcile(&repo, &["--repo", &other_str]);
    let entry = result_for(&report, "other_repo_memory");
    assert_eq!(claim_status(entry, "src/lib.rs"), "ok");

    let default_report = reconcile(&repo, &[]);
    let default_entry = result_for(&default_report, "other_repo_memory");
    assert_eq!(claim_status(default_entry, "src/lib.rs"), "missing");
    fs::remove_dir_all(&other).ok();
}
