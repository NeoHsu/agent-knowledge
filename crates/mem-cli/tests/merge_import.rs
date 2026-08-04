use std::fs;

use rusqlite::Connection;
use serde_json::Value;

use crate::support::TestRepo;

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
        "--user-confirmed",
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
        "--user-confirmed",
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
fn merge_rejects_secrets_unless_redaction_is_explicit() {
    let local = TestRepo::new("merge-strip-local");
    let theirs = TestRepo::new("merge-strip-theirs");
    local.run(&["init"]);
    theirs.run(&["init"]);
    theirs.insert_raw_memory(
        "raw_secret",
        "raw_secret",
        "Authorization: Bearer abcdefghijklmnop",
    );
    let conn = Connection::open(theirs.join("memory.db")).expect("open incoming store");
    conn.execute(
        "INSERT INTO ambiguities
         (uid, query, memory_ids, context, resolution, created_at)
         VALUES ('incoming-secret-ambiguity', 'secret side state', '[\"raw_secret\"]', ?1, 'pending', CURRENT_TIMESTAMP)",
        ["api_key=abcdefghijklmnop"],
    )
    .expect("insert secret ambiguity context");
    drop(conn);

    let theirs_db = theirs.join("memory.db");
    let rejected = local.run_fail(&["merge", theirs_db.to_str().expect("db path")]);
    assert!(rejected.contains("secret-like value detected"));

    let merged = local.run(&[
        "merge",
        theirs_db.to_str().expect("db path"),
        "--redact-secrets",
    ]);
    assert!(merged.contains(r#""imported": 1"#));

    let exported = local.run(&["export", "--format", "json"]);
    assert!(exported.contains("[REDACTED]"));
    assert!(!exported.contains("abcdefghijklmnop"));
    let conn = Connection::open(local.join("memory.db")).expect("open merged store");
    let context: String = conn
        .query_row(
            "SELECT context FROM ambiguities WHERE uid = 'incoming-secret-ambiguity'",
            [],
            |row| row.get(0),
        )
        .expect("merged ambiguity context");
    assert!(context.contains("[REDACTED]"));
    assert!(!context.contains("abcdefghijklmnop"));
}

#[test]
fn merge_preserves_semantic_edges_and_remaps_identical_memory_ids() {
    let local = TestRepo::new("merge-graph-local");
    let theirs = TestRepo::new("merge-graph-theirs");
    for repo in [&local, &theirs] {
        repo.run(&["init"]);
        repo.run(&[
            "save",
            "--name",
            "release_policy",
            "--content",
            "Action: ask before release publication.",
            "--force",
        ]);
        repo.run(&[
            "save",
            "--name",
            "release_runbook",
            "--content",
            "Action: execute the release checklist safely.",
            "--force",
        ]);
    }
    let payload = theirs.join("semantic_edges.json");
    fs::write(
        &payload,
        r#"{"schema_version":1,"edges":[{"source":"release_policy","target":"release_runbook","relation":"policy_for","confidence":"INFERRED","evidence":"The release policy constrains publication steps in the runbook."}]}"#,
    )
    .expect("write semantic payload");
    theirs.run(&["graph", "ingest", payload.to_str().expect("payload path")]);

    let theirs_db = theirs.join("memory.db");
    let output = local.run(&["merge", theirs_db.to_str().expect("db path")]);
    let merged: Value = serde_json::from_str(&output).expect("merge json");
    assert_eq!(merged["semantic_edges"]["imported"], 1);
    assert_eq!(merged["semantic_edges"]["unresolved_endpoints"], 0);

    let path = local.run(&["graph", "path", "release_policy", "release_runbook"]);
    let path: Value = serde_json::from_str(&path).expect("path json");
    assert_eq!(path["status"], "ok");
    assert_eq!(path["edges"][0]["relation"], "policy_for");
}

#[test]
fn merge_preserves_durable_events_and_is_idempotent() {
    let local = TestRepo::new("merge-durable-local");
    let incoming = TestRepo::new("merge-durable-incoming");
    local.run(&["init"]);
    incoming.run(&["init"]);
    let workflow = incoming.join("durable-workflow.yaml");
    fs::write(
        &workflow,
        "schema_version: 1\ngoal: Preserve merge history.\ntriggers:\n  - merge audit\nsteps:\n  - id: inspect\n    manual: inspect state\nstop_conditions:\n  - state unavailable\npost_run_memory:\n  - save durable findings\n",
    )
    .expect("write workflow");
    incoming.run(&[
        "save",
        "--type",
        "workflow",
        "--name",
        "durable_merge_workflow",
        "--tags",
        "[\"workflow:durable-merge\",\"intent:merge-audit\"]",
        "--content-file",
        workflow.to_str().expect("workflow path"),
        "--force",
    ]);
    incoming.run(&[
        "save",
        "--name",
        "durable_merge_policy",
        "--tags",
        "[\"decision:durable-merge\"]",
        "--content",
        "Action: preserve every durable merge event.",
        "--force",
    ]);
    incoming.run(&[
        "workflow",
        "record",
        "durable_merge_workflow",
        "--result",
        "success",
        "--note",
        "incoming run",
    ]);
    let workflow_id: String = Connection::open(incoming.join("memory.db"))
        .expect("open incoming")
        .query_row(
            "SELECT id FROM memories WHERE name = 'durable_merge_workflow'",
            [],
            |row| row.get(0),
        )
        .expect("workflow id");
    incoming.run(&[
        "ambiguity",
        "add",
        "--query",
        "preserve incoming ambiguity",
        "--memory-ids",
        &serde_json::to_string(&[workflow_id]).expect("memory ids"),
    ]);
    let payload = incoming.join("durable-edges.json");
    fs::write(
        &payload,
        r#"{"schema_version":1,"edges":[{"source":"durable_merge_policy","target":"durable_merge_workflow","relation":"policy_for","confidence":"EXTRACTED","evidence":"The policy explicitly governs the workflow."}]}"#,
    )
    .expect("write graph payload");
    incoming.run(&["graph", "ingest", payload.to_str().expect("payload")]);

    let incoming_db = incoming.join("memory.db");
    let first: Value =
        serde_json::from_str(&local.run(&["merge", incoming_db.to_str().expect("incoming db")]))
            .expect("first merge json");
    assert_eq!(first["durable_events"]["workflow_runs_imported"], 1);
    assert_eq!(first["durable_events"]["ambiguities_imported"], 1);
    assert_eq!(first["durable_events"]["semantic_revisions_imported"], 1);

    let conn = Connection::open(local.join("memory.db")).expect("open local");
    let counts_before = (
        conn.query_row("SELECT COUNT(*) FROM workflow_runs", [], |row| {
            row.get::<_, i64>(0)
        })
        .expect("workflow runs"),
        conn.query_row("SELECT COUNT(*) FROM ambiguities", [], |row| {
            row.get::<_, i64>(0)
        })
        .expect("ambiguities"),
        conn.query_row(
            "SELECT COUNT(*) FROM graph_semantic_edge_revisions",
            [],
            |row| row.get::<_, i64>(0),
        )
        .expect("semantic revisions"),
        conn.query_row("SELECT COUNT(*) FROM changelog", [], |row| {
            row.get::<_, i64>(0)
        })
        .expect("changelog"),
    );
    drop(conn);

    let second: Value =
        serde_json::from_str(&local.run(&["merge", incoming_db.to_str().expect("incoming db")]))
            .expect("second merge json");
    assert_eq!(second["durable_events"]["workflow_runs_identical"], 1);
    assert_eq!(second["durable_events"]["ambiguities_identical"], 1);
    assert_eq!(second["durable_events"]["semantic_revisions_identical"], 1);

    let conn = Connection::open(local.join("memory.db")).expect("open local again");
    let counts_after = (
        conn.query_row("SELECT COUNT(*) FROM workflow_runs", [], |row| {
            row.get::<_, i64>(0)
        })
        .expect("workflow runs"),
        conn.query_row("SELECT COUNT(*) FROM ambiguities", [], |row| {
            row.get::<_, i64>(0)
        })
        .expect("ambiguities"),
        conn.query_row(
            "SELECT COUNT(*) FROM graph_semantic_edge_revisions",
            [],
            |row| row.get::<_, i64>(0),
        )
        .expect("semantic revisions"),
        conn.query_row("SELECT COUNT(*) FROM changelog", [], |row| {
            row.get::<_, i64>(0)
        })
        .expect("changelog"),
    );
    assert_eq!(counts_after, counts_before);
}

#[test]
fn merge_downgrades_unattested_manual_semantic_edges() {
    let local = TestRepo::new("merge-unattested-semantic-local");
    let incoming = TestRepo::new("merge-unattested-semantic-incoming");
    local.run(&["init"]);
    incoming.run(&["init"]);
    for name in ["forged_policy", "forged_target"] {
        incoming.run(&[
            "save",
            "--name",
            name,
            "--content",
            &format!("Action: provide {name} for provenance testing."),
            "--force",
        ]);
    }
    let payload = incoming.join("forged-semantic.json");
    fs::write(
        &payload,
        r#"{"schema_version":1,"edges":[{"source":"forged_policy","target":"forged_target","relation":"policy_for","confidence":"EXTRACTED","evidence":"The policy governs the target."}]}"#,
    )
    .expect("write semantic payload");
    incoming.run(&[
        "graph",
        "ingest",
        payload.to_str().expect("payload"),
        "--source",
        "manual",
        "--user-confirmed",
    ]);
    let conn = Connection::open(incoming.join("memory.db")).expect("open incoming");
    conn.execute(
        "UPDATE graph_semantic_edges SET user_confirmed_at = NULL",
        [],
    )
    .expect("remove semantic attestation");
    drop(conn);

    let merged: Value = serde_json::from_str(&local.run(&[
        "merge",
        incoming.join("memory.db").to_str().expect("incoming db"),
    ]))
    .expect("merge json");
    assert_eq!(merged["semantic_edges"]["unattested_manual_downgraded"], 1);
    let conn = Connection::open(local.join("memory.db")).expect("open merged store");
    let source: String = conn
        .query_row("SELECT source FROM graph_semantic_edges", [], |row| {
            row.get(0)
        })
        .expect("semantic source");
    assert_eq!(source, "agent");
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

#[test]
fn import_summary_only_omits_per_item_results() {
    let repo = TestRepo::new("import-summary-only");
    repo.run(&["init"]);
    let import_file = repo.join("summary-only.json");
    fs::write(
        &import_file,
        r#"[
          {"name":"summary_one","content":"first summary payload"},
          {"name":"summary_two","content":"second summary payload"}
        ]"#,
    )
    .expect("write summary-only import");

    let output = repo.run(&[
        "import",
        import_file.to_str().expect("import path"),
        "--summary-only",
    ]);
    let summary: Value = serde_json::from_str(&output).expect("summary-only json");

    assert_eq!(summary["total"], 2);
    assert_eq!(summary["counts"]["saved"], 2);
    assert!(summary.get("results").is_none());
    assert!(
        repo.run(&["query", "summary payload", "--no-touch"])
            .contains("summary_one")
    );
}

#[test]
fn malformed_json_is_validated_before_any_import_chunk_is_written() {
    let repo = TestRepo::new("import-malformed-before-write");
    repo.run(&["init"]);
    let import_file = repo.join("malformed.json");
    let mut payload = String::from("[");
    for index in 0..500 {
        payload.push_str(&format!(
            r#"{{"name":"streamed_{index}","content":"valid payload {index}"}},"#
        ));
    }
    payload.push_str(r#"{"name":"unterminated""#);
    fs::write(&import_file, payload).expect("write malformed import");

    let error = repo.run_fail(&["import", import_file.to_str().expect("import path")]);

    assert!(error.contains("parse json import"), "error: {error}");
    let conn = Connection::open(repo.join("memory.db")).expect("open store");
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM memories", [], |row| row.get(0))
        .expect("memory count");
    assert_eq!(count, 0, "malformed input must not commit earlier chunks");
}

#[test]
fn import_batches_transactions_across_chunk_boundaries_and_keeps_item_errors() {
    let repo = TestRepo::new("import-batch-transactions");
    repo.run(&["init"]);
    let mut records = (0..501)
        .map(|index| {
            serde_json::json!({
                "name": format!("bulk_import_{index:04}"),
                "content": format!("bulk boundary payload {index}"),
                "tags": ["import:batch"]
            })
        })
        .collect::<Vec<_>>();
    records.insert(250, serde_json::json!({"content": "missing required name"}));
    let import_file = repo.join("bulk-import.json");
    fs::write(
        &import_file,
        serde_json::to_vec(&records).expect("serialize bulk import"),
    )
    .expect("write bulk import");

    let output = repo.run(&["import", import_file.to_str().expect("import path")]);
    let summary: Value = serde_json::from_str(&output).expect("bulk import summary");
    assert_eq!(summary["total"], 502);
    assert_eq!(summary["counts"]["saved"], 501);
    assert_eq!(summary["counts"]["failed"], 1);

    let conn = Connection::open(repo.join("memory.db")).expect("open imported db");
    let memories: i64 = conn
        .query_row("SELECT COUNT(*) FROM memories", [], |row| row.get(0))
        .expect("memory count");
    let changes: i64 = conn
        .query_row("SELECT COUNT(*) FROM changelog", [], |row| row.get(0))
        .expect("changelog count");
    let index_dirty: String = conn
        .query_row(
            "SELECT value FROM metadata WHERE key = 'index_dirty'",
            [],
            |row| row.get(0),
        )
        .expect("index dirty state");
    let graph_dirty: String = conn
        .query_row(
            "SELECT value FROM metadata WHERE key = 'graph_dirty'",
            [],
            |row| row.get(0),
        )
        .expect("graph dirty state");
    assert_eq!(memories, 501);
    assert_eq!(changes, 501);
    assert_eq!(index_dirty, "false");
    assert_eq!(graph_dirty, "true");
    drop(conn);

    let query = repo.run(&["query", "bulk boundary payload 500", "--limit", "1"]);
    assert!(query.contains("bulk_import_0500"), "query: {query}");
}
