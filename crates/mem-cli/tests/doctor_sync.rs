use rusqlite::Connection;
use std::fs;
use std::path::Path;
use std::process::Command;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

mod support;

use support::{TestRuntimeStore, temp_path};

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
    assert_eq!(find_check(checks, "store_schema")["status"], "ok");
    assert_eq!(find_check(checks, "store")["status"], "ok");
    assert_eq!(find_check(checks, "shared.skill")["status"], "missing");
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
    assert_eq!(find_check(checks, "shared.skill")["status"], "ok");
    assert_eq!(find_check(checks, "claude-code.policy")["status"], "ok");
    assert_eq!(find_check(checks, "claude-code.skill")["status"], "ok");
    assert_eq!(
        find_check(checks, "claude-code.session_hook")["status"],
        "ok"
    );

    fs::remove_dir_all(base).ok();
}

#[test]
fn doctor_reports_only_active_memory_count() {
    let store = TestRuntimeStore::new("doctor-active-count");
    store.run(&["init"]);
    store.run(&[
        "save",
        "--name",
        "active_note",
        "--content",
        "Trigger: doctor active count test. Action: remain active. Why: regression coverage added on 2026-07-17.",
        "--force",
    ]);
    store.run(&[
        "save",
        "--name",
        "deleted_note",
        "--content",
        "Trigger: doctor deleted count test. Action: become inactive. Why: regression coverage added on 2026-07-17.",
        "--force",
    ]);
    store.run(&["delete", "deleted_note"]);

    let report: serde_json::Value =
        serde_json::from_str(&store.run(&["doctor"])).expect("doctor json");
    let detail = find_check(&report["checks"], "store")["detail"]
        .as_str()
        .expect("store detail");
    assert!(detail.ends_with(", 1 active memories"), "{detail}");
}

#[test]
fn doctor_reports_missing_index_without_recreating_it() {
    let store = TestRuntimeStore::new("doctor-missing-index");
    store.run(&["init"]);
    fs::remove_dir_all(store.home().join("index")).expect("remove index");

    let report: serde_json::Value =
        serde_json::from_str(&store.run(&["doctor"])).expect("doctor json");
    let index = find_check(&report["checks"], "index");
    assert_eq!(index["status"], "warn");
    assert!(
        index["detail"]
            .as_str()
            .is_some_and(|detail| detail.contains("missing marker"))
    );
    assert!(!store.home().join("index").exists());
}

#[cfg(unix)]
#[test]
fn doctor_reports_insecure_store_permissions() {
    let store = TestRuntimeStore::new("doctor-permissions");
    store.run(&["init"]);
    fs::set_permissions(store.home(), fs::Permissions::from_mode(0o755)).expect("chmod root");
    fs::set_permissions(
        store.home().join("memory.db"),
        fs::Permissions::from_mode(0o644),
    )
    .expect("chmod db");

    let report: serde_json::Value =
        serde_json::from_str(&store.run(&["doctor"])).expect("doctor json");
    let permissions = find_check(&report["checks"], "store_permissions");
    assert_eq!(permissions["status"], "warn");
    assert!(
        permissions["detail"]
            .as_str()
            .is_some_and(|detail| detail.contains("755") && detail.contains("644"))
    );
    assert!(
        permissions["fix"]
            .as_str()
            .is_some_and(|fix| fix.contains("0700") && fix.contains("0600"))
    );
}

#[test]
fn doctor_reports_sqlite_integrity_failure() {
    let store = TestRuntimeStore::new("doctor-integrity");
    store.run(&["init"]);
    let conn = Connection::open(store.home().join("memory.db")).expect("open store");
    conn.execute_batch(
        "PRAGMA writable_schema = ON;
         UPDATE sqlite_schema SET rootpage = 999999 WHERE type = 'table' AND name = 'memories';
         PRAGMA writable_schema = OFF;",
    )
    .expect("corrupt memories root page");
    drop(conn);

    let report: serde_json::Value =
        serde_json::from_str(&store.run(&["doctor"])).expect("doctor json");
    let integrity = find_check(&report["checks"], "store_integrity");
    assert_eq!(integrity["status"], "error");
    assert_ne!(integrity["detail"], "SQLite quick_check: ok");
}

#[test]
fn doctor_warns_when_v5_policy_content_has_drifted() {
    let store = TestRuntimeStore::new("doctor-policy-drift");
    store.run(&["init"]);
    let base = temp_path("doctor-policy-drift-base");
    fs::create_dir_all(&base).expect("base dir");
    let base_str = base.to_str().expect("base");
    store.run(&["setup", "claude-code", "--base-dir", base_str]);

    let target = base.join(".claude/CLAUDE.md");
    let drifted = fs::read_to_string(&target)
        .expect("policy")
        .replace("Reject secrets by default", "Accept secrets by default");
    fs::write(&target, drifted).expect("drift policy");

    let output = store.run(&[
        "doctor",
        "--platform",
        "claude-code",
        "--base-dir",
        base_str,
    ]);
    let report: serde_json::Value = serde_json::from_str(&output).expect("doctor json");
    let policy = find_check(&report["checks"], "claude-code.policy");
    assert_eq!(policy["status"], "warn");
    assert!(
        policy["detail"]
            .as_str()
            .expect("detail")
            .contains("drifted v5 policy")
    );

    fs::remove_dir_all(base).ok();
}

#[test]
fn doctor_warns_when_prime_hook_hides_failures() {
    let store = TestRuntimeStore::new("doctor-legacy-hook");
    store.run(&["init"]);
    let base = temp_path("doctor-legacy-hook-base");
    fs::create_dir_all(base.join(".claude")).expect("base dir");
    fs::write(
        base.join(".claude/settings.json"),
        r#"{
  "hooks": {
    "SessionStart": [
      {
        "hooks": [
          {
            "type": "command",
            "command": "mem prime 2>/dev/null || true"
          }
        ]
      }
    ]
  }
}
"#,
    )
    .expect("seed settings");

    let output = store.run(&[
        "doctor",
        "--platform",
        "claude-code",
        "--base-dir",
        base.to_str().expect("base"),
    ]);
    let report: serde_json::Value = serde_json::from_str(&output).expect("doctor json");
    let hook = find_check(&report["checks"], "claude-code.session_hook");
    assert_eq!(hook["status"], "warn");
    assert!(
        hook["detail"]
            .as_str()
            .expect("detail")
            .contains("hides `mem prime` failures")
    );

    fs::remove_dir_all(base).ok();
}

#[test]
fn doctor_errors_when_store_schema_is_newer_than_the_binary() {
    let store = TestRuntimeStore::new("doctor-newer-schema");
    store.run(&["init"]);
    let conn = Connection::open(store.home().join("memory.db")).expect("open store");
    conn.pragma_update(None, "user_version", 999)
        .expect("set newer schema");
    drop(conn);

    let output = store.run(&["doctor"]);
    let report: serde_json::Value = serde_json::from_str(&output).expect("doctor json");
    assert_eq!(report["status"], "error");
    let schema = find_check(&report["checks"], "store_schema");
    assert_eq!(schema["status"], "error");
    assert!(
        schema["detail"]
            .as_str()
            .expect("detail")
            .contains("newer than this binary supports")
    );

    assert_eq!(find_check(&report["checks"], "store")["status"], "warn");
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
fn sync_dry_run_rejects_secret_leakage_without_committing() {
    let store = TestRuntimeStore::new("sync-secret-gate");
    store.run(&["init"]);
    let root = store.home();
    git(root, &["init", "-b", "main"]);
    git(root, &["config", "user.email", "test@example.com"]);
    git(root, &["config", "user.name", "Test User"]);
    store.run(&[
        "save",
        "--name",
        "sync_secret_probe",
        "--content",
        "Action: create a safe sync probe.",
        "--force",
    ]);
    let conn = Connection::open(root.join("memory.db")).expect("open store");
    conn.execute(
        "UPDATE memories SET description = ?1 WHERE name = 'sync_secret_probe'",
        ["ghp_abcdefghijklmnop1234567890"],
    )
    .expect("inject secret");
    drop(conn);

    let error = store.run_fail(&["sync", "--dry-run"]);
    assert!(error.contains("secret-like value detected in memories.description"));
    assert!(
        git(root, &["rev-list", "--all", "--count"])
            .trim()
            .parse::<u64>()
            .is_ok_and(|count| count == 0)
    );
}

#[cfg(unix)]
#[test]
fn sync_rejects_stale_bundle_backup_without_scanning_or_staging_it() {
    let store = TestRuntimeStore::new("sync-stale-bundle-backup");
    store.run(&["init"]);
    git_init_store(store.home());
    let backup = store.home().join(".bundle-replace-backup-stale");
    fs::create_dir_all(&backup).expect("stale backup directory");
    fs::write(backup.join("old.txt"), "token: abcdefgh12345678\n").expect("stale backup content");

    let error = store.run_fail(&["sync", "--dry-run"]);
    assert!(error.contains("stale bundle replacement backup found"));
    assert_eq!(
        git(store.home(), &["rev-list", "--all", "--count"]).trim(),
        "0"
    );
    assert!(
        !git(store.home(), &["diff", "--cached", "--name-only"]).contains(".bundle-replace-backup")
    );
}

#[test]
fn sync_disables_repository_hooks_and_commit_signing() {
    let store = TestRuntimeStore::new("sync-no-hooks");
    store.run(&["init"]);
    git_init_store(store.home());
    git(store.home(), &["config", "commit.gpgsign", "true"]);
    let marker = store.home().join("hook-ran");
    let hook = store.home().join(".git/hooks/pre-commit");
    fs::write(
        &hook,
        format!("#!/bin/sh\ntouch '{}'\nexit 1\n", marker.display()),
    )
    .expect("write pre-commit hook");
    #[cfg(unix)]
    fs::set_permissions(&hook, fs::Permissions::from_mode(0o700)).expect("chmod hook");
    store.run(&[
        "save",
        "--name",
        "hook_safety",
        "--content",
        "Action: synchronize without executing repository hooks.",
        "--force",
    ]);

    let output = store.run(&["sync"]);
    assert!(output.contains("local_only"));
    assert!(!marker.exists(), "sync executed a repository hook");
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
    assert!(ignore.contains(".bundle-replace-backup-*"));

    // No changes on the second run.
    let output = store.run(&["sync"]);
    let report: serde_json::Value = serde_json::from_str(&output).expect("sync json");
    assert_eq!(report["committed"], false);
}

#[test]
fn sync_rejects_and_rolls_back_secret_bearing_remote_state() {
    let bare = temp_path("sync-secret-remote-bare");
    fs::create_dir_all(&bare).expect("bare dir");
    git(&bare, &["init", "--bare", "-b", "main"]);
    let bare_url = bare.to_str().expect("bare path").to_string();

    let store = TestRuntimeStore::new("sync-secret-local");
    store.run(&["init"]);
    git_init_store(store.home());
    git(store.home(), &["remote", "add", "origin", &bare_url]);
    store.run(&[
        "save",
        "--name",
        "safe_local_memory",
        "--content",
        "Action: retain this safe local state.",
        "--force",
    ]);
    store.run(&["sync", "--push"]);
    let before = git(store.home(), &["rev-parse", "HEAD"]);

    let attacker = temp_path("sync-secret-attacker");
    git(
        attacker.parent().expect("attacker parent"),
        &[
            "clone",
            &bare_url,
            attacker.to_str().expect("attacker path"),
        ],
    );
    git(&attacker, &["config", "user.email", "attacker@example.com"]);
    git(&attacker, &["config", "user.name", "Attacker"]);
    git(&attacker, &["config", "commit.gpgsign", "false"]);
    fs::write(
        attacker.join("leaked.txt"),
        "api_key=sk_test_abcdefghijklmnopqrstuvwxyz123456",
    )
    .expect("write leaked file");
    git(&attacker, &["add", "leaked.txt"]);
    git(&attacker, &["commit", "-m", "add unsafe state"]);
    git(&attacker, &["push", "origin", "main"]);

    let error = store.run_fail(&["sync"]);
    assert!(
        error.contains("secret-like value detected"),
        "error: {error}"
    );
    assert!(!store.home().join("leaked.txt").exists());
    assert_eq!(git(store.home(), &["rev-parse", "HEAD"]), before);
    assert!(
        store
            .run(&["query", "safe local state", "--no-touch"])
            .contains("safe_local_memory")
    );

    fs::remove_dir_all(attacker).ok();
    fs::remove_dir_all(bare).ok();
}

#[test]
fn sync_aborts_git_merge_when_semantic_database_merge_is_rejected() {
    let bare = temp_path("sync-semantic-rejection-bare");
    fs::create_dir_all(&bare).expect("bare dir");
    git(&bare, &["init", "--bare", "-b", "main"]);
    let bare_url = bare.to_str().expect("bare path").to_string();

    let remote_store = TestRuntimeStore::new("sync-semantic-rejection-remote");
    remote_store.run(&["init"]);
    git_init_store(remote_store.home());
    git(remote_store.home(), &["remote", "add", "origin", &bare_url]);
    remote_store.run(&[
        "save",
        "--name",
        "shared_baseline",
        "--content",
        "safe shared baseline",
        "--force",
    ]);
    remote_store.run(&["sync", "--push"]);

    let local_store = TestRuntimeStore::new("sync-semantic-rejection-local");
    fs::remove_dir_all(local_store.home()).expect("clear clone target");
    git(
        local_store.home().parent().expect("clone parent"),
        &[
            "clone",
            &bare_url,
            local_store.home().to_str().expect("clone target"),
        ],
    );
    git(
        local_store.home(),
        &["config", "user.email", "local@example.com"],
    );
    git(local_store.home(), &["config", "user.name", "Local"]);
    git(local_store.home(), &["config", "commit.gpgsign", "false"]);
    local_store.run(&[
        "save",
        "--name",
        "safe_local_divergence",
        "--content",
        "retain this safe local divergence",
        "--force",
    ]);
    local_store.run(&["sync"]);
    let before = git(local_store.home(), &["rev-parse", "HEAD"]);

    remote_store.run(&[
        "save",
        "--name",
        "unsafe_remote_divergence",
        "--content",
        "placeholder before malicious database mutation",
        "--force",
    ]);
    let conn = Connection::open(remote_store.home().join("memory.db")).expect("open remote db");
    conn.execute(
        "UPDATE memories SET content = ?1 WHERE name = 'unsafe_remote_divergence'",
        ["api_key=sk_test_abcdefghijklmnopqrstuvwxyz123456"],
    )
    .expect("inject remote secret");
    conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()))
        .expect("checkpoint malicious remote db");
    drop(conn);
    git(remote_store.home(), &["add", "memory.db"]);
    git(
        remote_store.home(),
        &["commit", "-m", "inject unsafe remote database state"],
    );
    git(remote_store.home(), &["push", "origin", "main"]);

    let error = local_store.run_fail(&["sync"]);
    assert!(error.contains("semantic database merge failed"), "{error}");
    assert!(error.contains("secret-like value detected"), "{error}");
    assert!(!local_store.home().join(".git/MERGE_HEAD").exists());
    assert!(!local_store.home().join(".mem-sync-theirs.db").exists());
    assert_eq!(git(local_store.home(), &["rev-parse", "HEAD"]), before);
    assert!(
        local_store
            .run(&["query", "safe local divergence", "--no-touch"])
            .contains("safe_local_divergence")
    );

    fs::remove_dir_all(bare).ok();
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
    let output = store_a.run(&["sync", "--push"]);
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
    let graph_payload = store_a.home().join("sync-semantic-edges.json");
    fs::write(
        &graph_payload,
        r#"{"schema_version":1,"edges":[{"source":"rule_from_a","target":"second_rule_from_a","relation":"same_theme","confidence":"INFERRED","evidence":"Both memories define pull request and release coordination policy."}]}"#,
    )
    .expect("write graph payload");
    store_a.run(&[
        "graph",
        "ingest",
        graph_payload.to_str().expect("graph payload"),
    ]);
    fs::remove_file(&graph_payload).expect("remove graph payload");
    store_a.run(&["sync", "--push"]);

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
    let output = store_b.run(&["sync", "--push"]);
    let report: serde_json::Value = serde_json::from_str(&output).expect("sync json");
    assert_eq!(report["status"], "synced", "report: {report}");
    assert_eq!(report["pulled"], true, "report: {report}");
    assert_eq!(report["merge"]["status"], "merged", "report: {report}");
    assert_eq!(report["merge"]["imported"], 1, "report: {report}");
    assert_eq!(
        report["merge"]["semantic_edges"]["imported"], 1,
        "report: {report}"
    );
    assert_eq!(report["pushed"], true, "report: {report}");

    // B now holds both sides' memories.
    let output = store_b.run(&["query", "--type", "feedback", "--no-touch"]);
    assert!(output.contains("second_rule_from_a"), "query: {output}");
    assert!(output.contains("rule_from_b"), "query: {output}");
    let graph_path: serde_json::Value =
        serde_json::from_str(&store_b.run(&["graph", "path", "rule_from_a", "second_rule_from_a"]))
            .expect("graph path json");
    assert_eq!(graph_path["status"], "ok");
    assert_eq!(graph_path["edges"][0]["relation"], "same_theme");

    // A fresh clone must see the fully checkpointed semantic merge from B's commit,
    // not only B's local WAL state.
    let store_c = TestRuntimeStore::new("sync-machine-c");
    fs::remove_dir_all(store_c.home()).expect("clear verification clone target");
    git(
        store_c.home().parent().expect("parent"),
        &[
            "clone",
            &bare_url,
            store_c.home().to_str().expect("home path"),
        ],
    );
    store_c.run(&["reindex"]);
    let cloned = store_c.run(&["query", "--type", "feedback", "--no-touch"]);
    assert!(cloned.contains("second_rule_from_a"), "query: {cloned}");
    assert!(cloned.contains("rule_from_b"), "query: {cloned}");
    let cloned_graph: serde_json::Value =
        serde_json::from_str(&store_c.run(&["graph", "path", "rule_from_a", "second_rule_from_a"]))
            .expect("fresh clone graph path");
    assert_eq!(cloned_graph["status"], "ok");

    // A pulls B's memory on the next sync.
    let output = store_a.run(&["sync"]);
    let report: serde_json::Value = serde_json::from_str(&output).expect("sync json");
    assert_eq!(report["pulled"], true, "report: {report}");
    let output = store_a.run(&["query", "--type", "feedback", "--no-touch"]);
    assert!(output.contains("rule_from_b"), "query: {output}");

    fs::remove_dir_all(bare).ok();
}
