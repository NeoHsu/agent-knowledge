use std::fs;

use rusqlite::Connection;

mod support;

use support::TestRepo;

fn json(output: &str) -> serde_json::Value {
    serde_json::from_str(output).expect("json output")
}

#[test]
fn read_commands_never_initialize_migrate_or_repair_implicitly() {
    let missing = TestRepo::new("read-contract-missing");
    let error = missing.run_fail(&["query", "anything", "--no-touch"]);
    assert!(error.contains("run `mem init` explicitly"));
    assert!(!missing.join("memory.db").exists());

    let old = TestRepo::new("read-contract-old-schema");
    old.run(&["init"]);
    let conn = Connection::open(old.join("memory.db")).expect("open old store");
    conn.pragma_update(None, "user_version", 4)
        .expect("set old schema");
    drop(conn);
    let error = old.run_fail(&["prime"]);
    assert!(error.contains("requires explicit migration"));
    let conn = Connection::open(old.join("memory.db")).expect("reopen old store");
    let version: i64 = conn
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .expect("schema version");
    assert_eq!(version, 4);
    assert!(!old.join("memory.db.backup").exists());

    let missing_index = TestRepo::new("read-contract-missing-index");
    missing_index.run(&["init"]);
    missing_index.run(&[
        "save",
        "--name",
        "missing_index_probe",
        "--content",
        "Action: never recreate an index during a read.",
        "--force",
    ]);
    fs::remove_dir_all(missing_index.join("index")).expect("remove index");
    let before = fs::read(missing_index.join("memory.db")).expect("db before missing index query");
    let error = missing_index.run_fail(&["query", "recreate", "--no-touch"]);
    assert!(error.contains("missing marker"), "error: {error}");
    assert!(!missing_index.join("index").exists());
    assert_eq!(
        fs::read(missing_index.join("memory.db")).expect("db after missing index query"),
        before,
        "read-only query changed memory.db while the index was missing"
    );
    assert!(missing_index
        .run(&["query", "recreate", "--repair-index"])
        .contains("missing_index_probe"));

    let stale = TestRepo::new("read-contract-stale-index");
    stale.run(&["init"]);
    stale.run(&[
        "save",
        "--name",
        "stale_probe",
        "--content",
        "Action: preserve read-only behavior.",
        "--force",
    ]);
    let conn = Connection::open(stale.join("memory.db")).expect("open stale store");
    conn.execute(
        "INSERT INTO metadata (key, value, updated_at)
         VALUES ('index_dirty', 'true', CURRENT_TIMESTAMP)
         ON CONFLICT(key) DO UPDATE SET value = 'true', updated_at = CURRENT_TIMESTAMP",
        [],
    )
    .expect("mark index dirty");
    drop(conn);
    let before = fs::read(stale.join("memory.db")).expect("read db before");
    let error = stale.run_fail(&["query", "stale", "--no-touch"]);
    assert!(error.contains("search index is stale"));
    let after = fs::read(stale.join("memory.db")).expect("read db after");
    assert_eq!(after, before, "read-only query changed memory.db bytes");
}

#[test]
fn expired_memories_are_excluded_consistently_and_timestamps_are_strict() {
    let repo = TestRepo::new("memory-expiry-contract");
    repo.run(&["init"]);
    repo.run(&[
        "save",
        "--name",
        "expired_contract",
        "--expires-at",
        "2020-01-01T00:00:00Z",
        "--tags",
        "[\"test:expiry\"]",
        "--content",
        "Action: never return this expired memory.",
        "--force",
    ]);

    assert!(!repo
        .run(&["query", "expired contract"])
        .contains("expired_contract"));
    assert!(repo
        .run(&["query", "expired contract", "--expired"])
        .contains("expired_contract"));
    assert!(!repo.run(&["prime"]).contains("expired_contract"));
    let retro = json(&repo.run(&["retro", "daily"]));
    assert!(!retro["active_memories"]
        .as_array()
        .expect("active memories")
        .iter()
        .any(|memory| memory["name"] == "expired_contract"));

    repo.run(&[
        "update",
        "expired_contract",
        "--scope",
        "global",
        "--clear-expires-at",
        "--expected-version",
        "1",
    ]);
    assert!(repo
        .run(&["query", "expired contract"])
        .contains("expired_contract"));

    let invalid = repo.run_fail(&[
        "save",
        "--name",
        "invalid_expiry",
        "--expires-at",
        "not-a-time",
        "--content",
        "Action: reject malformed timestamps.",
    ]);
    assert!(invalid.contains("invalid RFC3339 timestamp"));
}

#[test]
fn secret_writes_are_rejected_unless_redaction_is_explicit() {
    let repo = TestRepo::new("secret-contract");
    repo.run(&["init"]);
    let secret = "ghp_abcdefghijklmnop1234567890";
    let rejected = repo.run_fail(&[
        "save",
        "--name",
        "secret_rejected",
        "--description",
        &format!("description {secret}"),
        "--content",
        "Action: reject the write.",
    ]);
    assert!(rejected.contains("secret-like value detected in description"));

    repo.run(&[
        "save",
        "--name",
        "secret_redacted",
        "--description",
        &format!("description {secret}"),
        "--tags",
        &format!("[\"token:{secret}\"]"),
        "--content",
        &format!("Action: redact content {secret}"),
        "--redact-secrets",
        "--force",
    ]);
    let exported = repo.run(&["export", "--format", "json"]);
    assert!(exported.contains("[REDACTED]"));
    assert!(!exported.contains(secret));

    let manual = repo.run_fail(&[
        "save",
        "--name",
        "manual_without_attestation",
        "--source",
        "manual",
        "--content",
        "Action: require explicit confirmation.",
    ]);
    assert!(manual.contains("source=manual requires --user-confirmed"));
    repo.run(&[
        "save",
        "--name",
        "manual_attested",
        "--source",
        "manual",
        "--user-confirmed",
        "--content",
        "Action: preserve an explicitly confirmed fact.",
        "--force",
    ]);
    let conn = Connection::open(repo.join("memory.db")).expect("open store");
    let confirmed: Option<String> = conn
        .query_row(
            "SELECT user_confirmed_at FROM memories WHERE name = 'manual_attested'",
            [],
            |row| row.get(0),
        )
        .expect("confirmation timestamp");
    assert!(confirmed.is_some());
}

#[test]
fn memory_names_are_scoped_and_mutations_resolve_explicitly() {
    let repo = TestRepo::new("scoped-name-contract");
    repo.run(&["init"]);
    repo.run(&[
        "save",
        "--name",
        "release_policy",
        "--scope",
        "global",
        "--content",
        "Action: apply the global release policy.",
        "--force",
    ]);
    repo.run(&[
        "save",
        "--name",
        "release_policy",
        "--scope",
        "project:example/app",
        "--tags",
        "[\"project:example/app\"]",
        "--content",
        "Action: apply the project release policy.",
        "--force",
    ]);

    repo.run(&[
        "update",
        "release_policy",
        "--scope",
        "project:example/app",
        "--content",
        "Action: apply the updated project release policy.",
        "--expected-version",
        "1",
    ]);
    let conn = Connection::open(repo.join("memory.db")).expect("open scoped store");
    let rows: Vec<(String, String)> = conn
        .prepare("SELECT scope, content FROM memories WHERE name = 'release_policy' ORDER BY scope")
        .expect("prepare scoped query")
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .expect("query scoped rows")
        .collect::<rusqlite::Result<Vec<_>>>()
        .expect("collect scoped rows");
    assert_eq!(rows.len(), 2);
    assert!(rows
        .iter()
        .any(|(scope, content)| scope == "global" && content.contains("global")));
    assert!(rows.iter().any(|(scope, content)| {
        scope == "project:example/app" && content.contains("updated project")
    }));
}

#[test]
fn durable_side_state_applies_secret_and_manual_source_gates() {
    let repo = TestRepo::new("side-state-security-contract");
    repo.run(&["init"]);
    let workflow = repo.join("secure-workflow.yaml");
    fs::write(
        &workflow,
        "schema_version: 1\ngoal: Validate side-state security.\ntriggers:\n  - security check\nsteps:\n  - id: inspect\n    manual: inspect durable state\nstop_conditions:\n  - unsafe input\n",
    )
    .expect("write workflow");
    repo.run(&[
        "save",
        "--type",
        "workflow",
        "--name",
        "secure_side_state",
        "--tags",
        "[\"workflow:security\"]",
        "--content-file",
        workflow.to_str().expect("workflow path"),
        "--force",
    ]);
    let secret = "ghp_abcdefghijklmnop1234567890";
    let rejected_run = repo.run_fail(&[
        "workflow",
        "record",
        "secure_side_state",
        "--result",
        "success",
        "--note",
        &format!("leaked {secret}"),
    ]);
    assert!(rejected_run.contains("secret-like value detected in workflow run note"));
    let manual_run = repo.run_fail(&[
        "workflow",
        "record",
        "secure_side_state",
        "--result",
        "success",
        "--source",
        "manual",
    ]);
    assert!(manual_run.contains("source=manual requires --user-confirmed"));
    repo.run(&[
        "workflow",
        "record",
        "secure_side_state",
        "--result",
        "success",
        "--note",
        &format!("redact {secret}"),
        "--redact-secrets",
    ]);

    let workflow_id: String = Connection::open(repo.join("memory.db"))
        .expect("open store")
        .query_row(
            "SELECT id FROM memories WHERE name = 'secure_side_state'",
            [],
            |row| row.get(0),
        )
        .expect("workflow id");
    let rejected_ambiguity = repo.run_fail(&[
        "ambiguity",
        "add",
        "--query",
        "side-state security",
        "--memory-ids",
        &serde_json::to_string(&[&workflow_id]).expect("memory ids"),
        "--context",
        &format!("leaked {secret}"),
    ]);
    assert!(rejected_ambiguity.contains("secret-like value detected in ambiguity context"));
    let added = json(&repo.run(&[
        "ambiguity",
        "add",
        "--query",
        "side-state security",
        "--memory-ids",
        &serde_json::to_string(&[&workflow_id]).expect("memory ids"),
        "--context",
        &format!("redact {secret}"),
        "--redact-secrets",
    ]));
    let ambiguity_id = added["id"].as_i64().expect("ambiguity id");
    let rejected_resolution = repo.run_fail(&[
        "ambiguity",
        "resolve",
        &ambiguity_id.to_string(),
        "--note",
        &format!("leaked {secret}"),
    ]);
    assert!(rejected_resolution.contains("secret-like value detected in ambiguity resolution note"));
    repo.run(&[
        "ambiguity",
        "resolve",
        &ambiguity_id.to_string(),
        "--note",
        &format!("redact {secret}"),
        "--redact-secrets",
    ]);

    let conn = Connection::open(repo.join("memory.db")).expect("reopen store");
    let note: String = conn
        .query_row("SELECT note FROM workflow_runs", [], |row| row.get(0))
        .expect("workflow note");
    let context: String = conn
        .query_row(
            "SELECT context FROM ambiguities WHERE id = ?1",
            [ambiguity_id],
            |row| row.get(0),
        )
        .expect("ambiguity context");
    let resolution: String = conn
        .query_row(
            "SELECT resolution FROM ambiguities WHERE id = ?1",
            [ambiguity_id],
            |row| row.get(0),
        )
        .expect("ambiguity resolution");
    for durable_value in [note, context, resolution] {
        assert!(durable_value.contains("[REDACTED]"));
        assert!(!durable_value.contains(secret));
    }
}

#[test]
fn commands_reject_unexpected_sqlite_schema_objects() {
    let repo = TestRepo::new("schema-object-contract");
    repo.run(&["init"]);
    let conn = Connection::open(repo.join("memory.db")).expect("open store");
    conn.execute_batch(
        "CREATE TRIGGER unexpected_local_trigger
         AFTER INSERT ON memories BEGIN DELETE FROM memories; END;",
    )
    .expect("install unexpected trigger");
    drop(conn);

    let error = repo.run_fail(&["query", "anything", "--no-touch"]);
    assert!(
        error.contains("unexpected schema objects"),
        "error: {error}"
    );
    let doctor: serde_json::Value =
        serde_json::from_str(&repo.run(&["doctor"])).expect("doctor json");
    let compatibility = doctor["checks"]
        .as_array()
        .expect("checks")
        .iter()
        .find(|check| check["id"] == "store_compatibility")
        .expect("compatibility check");
    assert_eq!(compatibility["status"], "error");
}

#[test]
fn memory_writes_enforce_resource_limits() {
    let repo = TestRepo::new("memory-resource-contract");
    repo.run(&["init"]);
    let oversized_name = "n".repeat(257);
    let name_error = repo.run_fail(&[
        "save",
        "--name",
        &oversized_name,
        "--content",
        "Action: reject oversized names.",
    ]);
    assert!(name_error.contains("memory name must be between 1 and 256 bytes"));

    let oversized_content = repo.join("oversized-memory.txt");
    fs::write(&oversized_content, vec![b'x'; 1_048_577]).expect("write oversized content");
    let content_error = repo.run_fail(&[
        "save",
        "--name",
        "oversized_content",
        "--content-file",
        oversized_content.to_str().expect("content path"),
    ]);
    assert!(content_error.contains("memory content file exceeds 1048576 bytes"));

    let tags = (0..101)
        .map(|index| format!("tag:{index}"))
        .collect::<Vec<_>>();
    let tag_json = serde_json::to_string(&tags).expect("tags json");
    let tag_error = repo.run_fail(&[
        "save",
        "--name",
        "oversized_tags",
        "--tags",
        &tag_json,
        "--content",
        "Action: reject excessive tag fanout.",
    ]);
    assert!(tag_error.contains("memory tags cannot exceed 100 entries"));
}

#[test]
fn semantic_ingest_is_strict_and_persists_expiry() {
    let repo = TestRepo::new("semantic-contract");
    repo.run(&["init"]);
    for name in ["semantic_source", "semantic_target"] {
        repo.run(&[
            "save",
            "--name",
            name,
            "--content",
            &format!("Action: use {name} for semantic validation."),
            "--force",
        ]);
    }
    let payload = repo.join("expired-edge.json");
    fs::write(
        &payload,
        r#"{"schema_version":1,"edges":[{"source":"semantic_source","target":"semantic_target","relation":"depends_on","confidence":"EXTRACTED","evidence":"The source explicitly depends on the target.","valid_until":"2020-01-01T00:00:00Z"}]}"#,
    )
    .expect("write semantic payload");
    let ingested = json(&repo.run(&["graph", "ingest", payload.to_str().expect("payload path")]));
    assert_eq!(ingested["inserted"], 1);
    let conn = Connection::open(repo.join("memory.db")).expect("open graph store");
    let valid_until: Option<String> = conn
        .query_row("SELECT valid_until FROM graph_semantic_edges", [], |row| {
            row.get(0)
        })
        .expect("semantic expiry");
    assert_eq!(valid_until.as_deref(), Some("2020-01-01T00:00:00+00:00"));
    drop(conn);
    let path = json(&repo.run(&[
        "graph",
        "path",
        "semantic_source",
        "semantic_target",
        "--scope",
        "all",
    ]));
    assert_eq!(path["status"], "not_found");

    let unknown = repo.join("unknown-edge-field.json");
    fs::write(
        &unknown,
        r#"{"schema_version":1,"edges":[{"source":"semantic_source","target":"semantic_target","relation":"depends_on","confidence":"EXTRACTED","evidence":"Evidence.","typo_field":true}]}"#,
    )
    .expect("write unknown field payload");
    let error = repo.run_fail(&[
        "graph",
        "ingest",
        unknown.to_str().expect("unknown payload"),
    ]);
    assert!(error.contains("unknown field"));
}
