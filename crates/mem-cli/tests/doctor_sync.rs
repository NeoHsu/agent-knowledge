use std::fs;
use std::path::Path;
use std::process::Command;

mod support;

use support::{temp_path, TestRuntimeStore};

fn git(dir: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .expect("run git");
    assert!(
        output.status.success(),
        "git {:?} failed: {}",
        args,
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).to_string()
}

fn git_init_store(dir: &Path) {
    git(dir, &["init", "-b", "main"]);
    git(dir, &["config", "user.email", "test@example.com"]);
    git(dir, &["config", "user.name", "Test"]);
    git(dir, &["config", "commit.gpgsign", "false"]);
}

fn find_check<'a>(checks: &'a serde_json::Value, id: &str) -> &'a serde_json::Value {
    checks
        .as_array()
        .expect("checks array")
        .iter()
        .find(|entry| entry["id"] == id)
        .unwrap_or_else(|| panic!("check {id} missing: {checks}"))
}

#[test]
fn doctor_reports_store_and_platform_wiring() {
    let store = TestRuntimeStore::new("doctor-basic");
    store.run(&["init"]);
    let base = temp_path("doctor-base");
    fs::create_dir_all(&base).expect("base dir");
    let base_str = base.to_str().expect("base");

    let output = store.run(&["doctor", "--base-dir", base_str]);
    let report: serde_json::Value = serde_json::from_str(&output).expect("doctor json");
    let checks = &report["checks"];
    assert_eq!(find_check(checks, "store")["status"], "ok");
    assert_eq!(find_check(checks, "store_git")["status"], "warn");
    assert_eq!(
        find_check(checks, "claude-code.policy")["status"],
        "missing"
    );

    store.run(&["setup", "claude-code", "--base-dir", base_str]);
    let output = store.run(&[
        "doctor",
        "--platform",
        "claude-code",
        "--base-dir",
        base_str,
    ]);
    let report: serde_json::Value = serde_json::from_str(&output).expect("doctor json");
    let checks = &report["checks"];
    assert_eq!(find_check(checks, "claude-code.policy")["status"], "ok");
    assert_eq!(find_check(checks, "claude-code.skill")["status"], "ok");
    assert_eq!(
        find_check(checks, "claude-code.session_hook")["status"],
        "ok"
    );

    fs::remove_dir_all(base).ok();
}

#[test]
fn doctor_without_store_errors() {
    let store = TestRuntimeStore::new("doctor-no-store");
    let base = temp_path("doctor-no-store-base");
    fs::create_dir_all(&base).expect("base dir");

    let output = store.run(&["doctor", "--base-dir", base.to_str().expect("base")]);
    let report: serde_json::Value = serde_json::from_str(&output).expect("doctor json");
    assert_eq!(report["status"], "error");
    assert_eq!(find_check(&report["checks"], "store")["status"], "error");

    fs::remove_dir_all(base).ok();
}

#[test]
fn sync_requires_git_repository() {
    let store = TestRuntimeStore::new("sync-no-git");
    store.run(&["init"]);
    let output = store.run_fail(&["sync"]);
    assert!(output.contains("not a git repository"), "message: {output}");
}

#[test]
fn sync_commits_locally_without_remote() {
    let store = TestRuntimeStore::new("sync-local");
    store.run(&["init"]);
    git_init_store(store.home());
    store.run(&[
        "save",
        "--type",
        "feedback",
        "--name",
        "local_rule",
        "--tags",
        "[\"domain:test\"]",
        "--content",
        "a durable local rule",
    ]);

    let output = store.run(&["sync"]);
    let report: serde_json::Value = serde_json::from_str(&output).expect("sync json");
    assert_eq!(report["status"], "local_only");
    assert_eq!(report["committed"], true);

    let ignore = fs::read_to_string(store.home().join(".gitignore")).expect("gitignore");
    assert!(ignore.contains("index/"));
    assert!(ignore.contains(".mem.lock"));

    // No changes on the second run.
    let output = store.run(&["sync"]);
    let report: serde_json::Value = serde_json::from_str(&output).expect("sync json");
    assert_eq!(report["committed"], false);
}

#[test]
fn sync_pushes_and_semantically_merges_remote_changes() {
    let bare = temp_path("sync-remote-bare");
    fs::create_dir_all(&bare).expect("bare dir");
    git(&bare, &["init", "--bare", "-b", "main"]);
    let bare_url = bare.to_str().expect("bare path").to_string();

    // Machine A: init, save, push.
    let store_a = TestRuntimeStore::new("sync-machine-a");
    store_a.run(&["init"]);
    git_init_store(store_a.home());
    git(store_a.home(), &["remote", "add", "origin", &bare_url]);
    store_a.run(&[
        "save",
        "--type",
        "feedback",
        "--name",
        "rule_from_a",
        "--tags",
        "[\"domain:test\"]",
        "--force",
        "--content",
        "PR 拆小，每個 PR 少於 400 行",
    ]);
    let output = store_a.run(&["sync"]);
    let report: serde_json::Value = serde_json::from_str(&output).expect("sync json");
    assert_eq!(report["status"], "synced");
    assert_eq!(report["pushed"], true);

    // Machine B: clone the store, then both sides write.
    let store_b = TestRuntimeStore::new("sync-machine-b");
    fs::remove_dir_all(store_b.home()).expect("clear clone target");
    git(
        store_b.home().parent().expect("parent"),
        &[
            "clone",
            &bare_url,
            store_b.home().to_str().expect("home path"),
        ],
    );
    git(store_b.home(), &["config", "user.email", "b@example.com"]);
    git(store_b.home(), &["config", "user.name", "Test B"]);
    git(store_b.home(), &["config", "commit.gpgsign", "false"]);

    store_a.run(&[
        "save",
        "--type",
        "feedback",
        "--name",
        "second_rule_from_a",
        "--tags",
        "[\"domain:test\"]",
        "--force",
        "--content",
        "release owner rotates weekly on Mondays",
    ]);
    store_a.run(&["sync"]);

    store_b.run(&[
        "save",
        "--type",
        "feedback",
        "--name",
        "rule_from_b",
        "--tags",
        "[\"domain:test\"]",
        "--force",
        "--content",
        "always run integration tests before deploying",
    ]);
    let output = store_b.run(&["sync"]);
    let report: serde_json::Value = serde_json::from_str(&output).expect("sync json");
    assert_eq!(report["status"], "synced", "report: {report}");
    assert_eq!(report["pulled"], true, "report: {report}");
    assert_eq!(report["merge"]["status"], "merged", "report: {report}");
    assert_eq!(report["merge"]["imported"], 1, "report: {report}");
    assert_eq!(report["pushed"], true, "report: {report}");

    // B now holds both sides' memories.
    let output = store_b.run(&["query", "--type", "feedback", "--no-touch"]);
    assert!(output.contains("second_rule_from_a"), "query: {output}");
    assert!(output.contains("rule_from_b"), "query: {output}");

    // A pulls B's memory on the next sync.
    let output = store_a.run(&["sync"]);
    let report: serde_json::Value = serde_json::from_str(&output).expect("sync json");
    assert_eq!(report["pulled"], true, "report: {report}");
    let output = store_a.run(&["query", "--type", "feedback", "--no-touch"]);
    assert!(output.contains("rule_from_b"), "query: {output}");

    fs::remove_dir_all(bare).ok();
}
