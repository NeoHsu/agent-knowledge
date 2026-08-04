use std::fs;

use rusqlite::Connection;

mod support;

use support::TestRepo;

fn parse_error(output: &str) -> serde_json::Value {
    serde_json::from_str(output.trim())
        .unwrap_or_else(|error| panic!("expected one JSON error object, got {output:?}: {error}"))
}

fn break_index(repo: &TestRepo) {
    fs::remove_dir_all(repo.join("index")).expect("remove index");
    fs::write(repo.join("index"), "not a directory").expect("break index path");
}

fn assert_committed_index_error(output: &str, operation: &str) -> serde_json::Value {
    let error = parse_error(output);
    assert_eq!(error["code"], "index_stale_after_write");
    assert_eq!(error["details"]["operation"], operation);
    assert_eq!(error["details"]["durable_write_committed"], true);
    assert_eq!(error["details"]["index_stale"], true);
    assert_eq!(error["details"]["recovery"], "mem reindex");
    assert_eq!(error["retryable"], false);
    error
}

#[test]
fn json_errors_classify_missing_stores() {
    let repo = TestRepo::new("json-runtime-error");

    let output = repo.run_fail(&["--json-errors", "query", "missing"]);
    let error = parse_error(&output);

    assert_eq!(error["status"], "error");
    assert_eq!(error["contract_version"], 1);
    assert_eq!(error["code"], "not_found");
    assert_eq!(error["exit_code"], 4);
    assert_eq!(error["retryable"], false);
    assert!(
        error["message"]
            .as_str()
            .expect("message")
            .contains("memory store not found")
    );
}

#[test]
fn json_errors_classify_core_failures() {
    let safety = TestRepo::new("json-safety-error");
    safety.run(&["init"]);
    let secret = ["ghp_", "abcdefghijklmnop1234567890"].concat();
    let output = safety.run_fail(&[
        "--json-errors",
        "save",
        "--name",
        "unsafe_value",
        "--content",
        &secret,
        "--force",
    ]);
    let error = parse_error(&output);
    assert_eq!(error["code"], "safety_violation");
    assert_eq!(error["exit_code"], 2);
    assert!(!output.contains(&secret));

    let usage = TestRepo::new("json-usage-error");
    usage.run(&["init"]);
    let output = usage.run_fail(&[
        "--json-errors",
        "save",
        "--name",
        "invalid_scope",
        "--scope",
        "not-a-scope",
        "--content",
        "invalid input",
        "--force",
    ]);
    let error = parse_error(&output);
    assert_eq!(error["code"], "usage");
    assert_eq!(error["exit_code"], 2);

    let compatibility = TestRepo::new("json-compatibility-error");
    compatibility.run(&["init"]);
    let connection = Connection::open(compatibility.join("memory.db")).expect("open store");
    connection
        .pragma_update(None, "user_version", 999)
        .expect("future schema");
    drop(connection);
    let output = compatibility.run_fail(&["--json-errors", "query"]);
    let error = parse_error(&output);
    assert_eq!(error["code"], "compatibility");
    assert_eq!(error["exit_code"], 2);

    let integrity = TestRepo::new("json-integrity-error");
    integrity.run(&["init"]);
    let connection = Connection::open(integrity.join("memory.db")).expect("open store");
    connection
        .execute("CREATE TABLE unexpected_private_state (id INTEGER)", [])
        .expect("unexpected table");
    drop(connection);
    let output = integrity.run_fail(&["--json-errors", "query"]);
    let error = parse_error(&output);
    assert_eq!(error["code"], "integrity");
    assert_eq!(error["exit_code"], 1);

    let conflict = TestRepo::new("json-conflict-error");
    conflict.run(&["init"]);
    fs::write(
        conflict.join("manifest.toml"),
        r#"version = 1

[artifacts.scripts.shared]
path = "artifacts/scripts/shared.sh"
kind = "script"
scope = "global"
checksum = "sha256:fixture"

[artifacts.templates.shared]
path = "artifacts/templates/shared.md"
kind = "template"
scope = "global"
checksum = "sha256:fixture"
"#,
    )
    .expect("ambiguous manifest");
    let output = conflict.run_fail(&["--json-errors", "artifact", "show", "shared"]);
    let error = parse_error(&output);
    assert_eq!(error["code"], "conflict");
    assert_eq!(error["exit_code"], 1);
}

#[test]
fn json_errors_distinguish_committed_writes_from_index_failures() {
    let repo = TestRepo::new("json-committed-index-error");
    repo.run(&["init"]);
    break_index(&repo);

    let output = repo.run_fail(&[
        "--json-errors",
        "save",
        "--name",
        "committed_before_index_failure",
        "--content",
        "Trigger: index failure. Action: preserve the durable write. Why: contract test.",
        "--force",
    ]);
    let error = assert_committed_index_error(&output, "memory save");

    assert_eq!(
        error["details"]["memory_id"],
        "committed_before_index_failure"
    );
    assert_eq!(error["details"]["version"], 1);
    assert!(
        error["message"]
            .as_str()
            .expect("message")
            .contains("durable memory save committed")
    );

    let connection = Connection::open(repo.join("memory.db")).expect("open store");
    let version: i64 = connection
        .query_row(
            "SELECT version FROM memories WHERE name = 'committed_before_index_failure'",
            [],
            |row| row.get(0),
        )
        .expect("committed memory");
    let index_dirty: String = connection
        .query_row(
            "SELECT value FROM metadata WHERE key = 'index_dirty'",
            [],
            |row| row.get(0),
        )
        .expect("index dirty metadata");
    assert_eq!(version, 1);
    assert_eq!(index_dirty, "true");
    drop(connection);

    fs::remove_file(repo.join("index")).expect("remove broken index path");
    repo.run(&["reindex"]);
    let queried = repo.run(&["query", "committed_before_index_failure"]);
    assert!(queried.contains("committed_before_index_failure"));
}

#[test]
fn committed_index_errors_cover_lifecycle_bulk_merge_and_bundle_writes() {
    let update = TestRepo::new("json-update-index-error");
    update.run(&["init"]);
    update.run(&[
        "save",
        "--name",
        "update_target",
        "--content",
        "before",
        "--force",
    ]);
    break_index(&update);
    let output = update.run_fail(&[
        "--json-errors",
        "update",
        "update_target",
        "--content",
        "after",
    ]);
    let error = assert_committed_index_error(&output, "memory update");
    assert_eq!(error["details"]["version"], 2);

    let supersede = TestRepo::new("json-supersede-index-error");
    supersede.run(&["init"]);
    supersede.run(&[
        "save",
        "--name",
        "old_memory",
        "--content",
        "old",
        "--force",
    ]);
    break_index(&supersede);
    let output = supersede.run_fail(&[
        "--json-errors",
        "supersede",
        "old_memory",
        "new_memory",
        "--content",
        "new",
    ]);
    assert_committed_index_error(&output, "memory supersede");

    let delete = TestRepo::new("json-delete-index-error");
    delete.run(&["init"]);
    delete.run(&[
        "save",
        "--name",
        "delete_target",
        "--content",
        "delete me",
        "--force",
    ]);
    break_index(&delete);
    let output = delete.run_fail(&["--json-errors", "delete", "delete_target"]);
    let error = assert_committed_index_error(&output, "soft memory delete");
    assert_eq!(error["details"]["version"], 2);

    let import = TestRepo::new("json-import-index-error");
    import.run(&["init"]);
    let import_file = import.join("memories.json");
    fs::write(
        &import_file,
        r#"[{"name":"imported_before_index_failure","content":"durable import"}]"#,
    )
    .expect("write import fixture");
    break_index(&import);
    let output = import.run_fail(&[
        "--json-errors",
        "import",
        import_file.to_str().expect("import path"),
    ]);
    let error = assert_committed_index_error(&output, "JSON import");
    assert_eq!(error["details"]["changed_count"], 1);

    let source = TestRepo::new("json-merge-source");
    source.run(&["init"]);
    source.run(&[
        "save",
        "--name",
        "merged_before_index_failure",
        "--content",
        "durable merge",
        "--force",
    ]);
    let merge = TestRepo::new("json-merge-index-error");
    merge.run(&["init"]);
    break_index(&merge);
    let output = merge.run_fail(&[
        "--json-errors",
        "merge",
        source.join("memory.db").to_str().expect("merge database"),
    ]);
    let error = assert_committed_index_error(&output, "database merge");
    assert_eq!(error["details"]["changed_count"], 1);

    let bundle_file = source.join("store.tgz");
    source.run(&[
        "bundle",
        "export",
        bundle_file.to_str().expect("bundle path"),
    ]);
    let bundle = TestRepo::new("json-bundle-index-error");
    fs::create_dir_all(bundle.path()).expect("bundle target root");
    fs::write(bundle.join("index"), "not a directory").expect("break bundle index path");
    let output = bundle.run_fail(&[
        "--json-errors",
        "bundle",
        "import",
        bundle_file.to_str().expect("bundle path"),
    ]);
    assert_committed_index_error(&output, "bundle import");
}

#[test]
fn json_errors_wrap_clap_parse_failures() {
    let repo = TestRepo::new("json-parse-error");

    let output = repo.run_fail(&["--json-errors", "not-a-command"]);
    let error = parse_error(&output);

    assert_eq!(error["status"], "error");
    assert_eq!(error["contract_version"], 1);
    assert_eq!(error["code"], "cli_parse_error");
    assert_eq!(error["exit_code"], 2);
    assert_eq!(error["retryable"], false);
    assert!(
        error["message"]
            .as_str()
            .expect("message")
            .contains("unrecognized subcommand")
    );
}

#[test]
fn json_errors_redact_secret_like_parse_input() {
    let repo = TestRepo::new("json-secret-error");
    let secret = ["ghp_", "abcdefghijklmnop1234567890"].concat();

    let output = repo.run_fail(&["--json-errors", &secret]);
    let error = parse_error(&output);

    assert_eq!(error["code"], "cli_parse_error");
    assert!(!output.contains(&secret));
    assert!(
        error["message"]
            .as_str()
            .expect("message")
            .contains("[REDACTED]")
    );
}

#[test]
fn default_errors_remain_human_readable() {
    let repo = TestRepo::new("human-error");

    let output = repo.run_fail(&["not-a-command"]);

    assert!(serde_json::from_str::<serde_json::Value>(output.trim()).is_err());
    assert!(output.contains("unrecognized subcommand"));
}

#[test]
fn json_errors_does_not_change_help_success_output() {
    let repo = TestRepo::new("json-help");

    let output = repo.run(&["--json-errors", "--help"]);

    assert!(output.contains("Portable agent memory CLI"));
    assert!(output.contains("--json-errors"));
}
