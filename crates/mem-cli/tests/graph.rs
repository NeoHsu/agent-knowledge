use std::fs;

use rusqlite::Connection;

mod support;

use support::TestRepo;

fn json(output: &str) -> serde_json::Value {
    serde_json::from_str(output).expect("json output")
}

fn save_reference(repo: &TestRepo, name: &str, tags: &str, content: &str) {
    repo.run(&["save", "--name", name, "--tags", tags, "--content", content]);
}

#[test]
fn graph_schema_is_created_on_init() {
    let repo = TestRepo::new("graph-schema");
    repo.run(&["init"]);
    let conn = Connection::open(repo.join("memory.db")).expect("open db");

    let version: i64 = conn
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .expect("user_version");
    assert!(version >= 4);

    for table in [
        "graph_nodes",
        "graph_edges",
        "graph_semantic_edges",
        "graph_semantic_edge_revisions",
    ] {
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
                [table],
                |row| row.get(0),
            )
            .expect("table exists query");
        assert_eq!(count, 1, "missing table {table}");
    }
}

#[test]
fn graph_rebuild_extracts_memory_metadata_claims_and_shared_tag_paths() {
    let repo = TestRepo::new("graph-memory");
    repo.run(&["init"]);
    save_reference(
        &repo,
        "release_policy",
        r#"["domain:release","risk:high"]"#,
        "Action: check `docs/release.md` and run `git status` before release.",
    );
    save_reference(
        &repo,
        "release_notes",
        r#"["domain:release"]"#,
        "Why: release notes use the same release domain.",
    );

    let rebuilt = json(&repo.run(&["graph", "rebuild"]));
    assert_eq!(rebuilt["status"], "rebuilt");
    assert!(rebuilt["nodes"].as_u64().expect("nodes") >= 7);
    assert!(rebuilt["edges"].as_u64().expect("edges") >= 8);

    let explained = json(&repo.run(&["graph", "explain", "release_policy"]));
    assert_eq!(explained["node"]["id"], "memory:release_policy");
    let neighbors = explained["neighbors"].as_array().expect("neighbors");
    assert!(neighbors.iter().any(|edge| edge["relation"] == "has_tag"));
    assert!(neighbors
        .iter()
        .any(|edge| edge["relation"] == "mentions_path"));
    assert!(neighbors
        .iter()
        .any(|edge| edge["relation"] == "mentions_command"));

    let path = json(&repo.run(&[
        "graph",
        "path",
        "release_policy",
        "release_notes",
        "--max-depth",
        "2",
    ]));
    assert_eq!(path["status"], "ok");
    assert_eq!(path["hops"], 2);
    assert!(path["nodes"]
        .as_array()
        .expect("path nodes")
        .iter()
        .any(|node| node["id"] == "tag:domain:release"));

    let exported = json(&repo.run(&["graph", "export", "--format", "json"]));
    assert_eq!(exported["schema_version"], 1);
    assert!(exported["nodes"]
        .as_array()
        .expect("export nodes")
        .iter()
        .any(|node| node["id"] == "memory:release_policy"));
    assert!(exported["edges"]
        .as_array()
        .expect("export edges")
        .iter()
        .any(|edge| edge["relation"] == "mentions_path"));
}

#[test]
fn graph_rebuild_extracts_workflow_artifacts_steps_and_runs() {
    let repo = TestRepo::new("graph-workflow");
    repo.run(&["init"]);
    let artifact_path = repo.join("artifacts/scripts/ci-triage.sh");
    fs::create_dir_all(artifact_path.parent().expect("artifact parent")).expect("artifact dir");
    fs::write(&artifact_path, "#!/usr/bin/env sh\necho ci\n").expect("artifact file");
    repo.run(&[
        "artifact",
        "add",
        "artifacts/scripts/ci-triage.sh",
        "--kind",
        "script",
        "--scope",
        "global",
        "--tags",
        r#"["tool:ci"]"#,
    ]);
    repo.run(&[
        "save",
        "--type",
        "workflow",
        "--name",
        "ci_triage_workflow",
        "--tags",
        r#"["workflow:ci-triage","intent:ci-triage","risk:medium"]"#,
        "--content",
        "schema_version: 1\ngoal: Triage CI.\ntriggers:\n  - ci fails\nreusable_scripts:\n  - path: artifacts/scripts/ci-triage.sh\n    owner: knowledge_store\n    required: true\nsteps:\n  - id: collect\n    run: artifacts/scripts/ci-triage.sh\n    confirm: true\nstop_conditions:\n  - missing artifact\n",
    ]);
    repo.run(&[
        "workflow",
        "record",
        "ci_triage_workflow",
        "--result",
        "failure",
        "--note",
        "ci unavailable",
    ]);

    let explained = json(&repo.run(&["graph", "explain", "ci_triage_workflow"]));
    let neighbors = explained["neighbors"].as_array().expect("neighbors");
    assert!(neighbors
        .iter()
        .any(|edge| edge["relation"] == "references_artifact"));
    assert!(neighbors
        .iter()
        .any(|edge| edge["relation"] == "has_workflow_step"));
    assert!(neighbors
        .iter()
        .any(|edge| edge["relation"] == "recorded_run"));

    let artifact_path = json(&repo.run(&[
        "graph",
        "path",
        "ci_triage_workflow",
        "artifact:artifacts/scripts/ci-triage.sh",
    ]));
    assert_eq!(artifact_path["status"], "ok");
    assert_eq!(artifact_path["hops"], 1);

    let step_explain = json(&repo.run(&[
        "graph",
        "explain",
        "workflow_step:ci_triage_workflow:collect",
    ]));
    assert!(step_explain["neighbors"]
        .as_array()
        .expect("step neighbors")
        .iter()
        .any(|edge| edge["relation"] == "requires_confirmation"));

    let shown = json(&repo.run(&[
        "workflow",
        "show",
        "ci_triage_workflow",
        "--with-graph-context",
    ]));
    assert!(shown["graph_context"]["neighbors"]
        .as_array()
        .expect("workflow graph context")
        .iter()
        .any(|edge| edge["relation"] == "references_artifact"));
}

#[test]
fn graph_candidates_emit_skill_extraction_payload() {
    let repo = TestRepo::new("graph-candidates");
    repo.run(&["init"]);
    save_reference(
        &repo,
        "assistant_style",
        r#"["style:output"]"#,
        "Action: keep assistant output concise.",
    );

    let candidates = json(&repo.run(&["graph", "candidates", "--scope", "all", "--limit", "10"]));
    assert_eq!(candidates["status"], "ok");
    assert!(candidates["allowed_relations"]
        .as_array()
        .expect("relations")
        .iter()
        .any(|relation| relation == "mentions_concept"));
    assert_eq!(candidates["memories"][0]["id"], "assistant_style");
}

#[test]
fn graph_path_honors_direction_and_uses_global_weighted_tie_breaks() {
    let repo = TestRepo::new("graph-direction-weight");
    repo.run(&["init"]);
    for name in ["path_a", "path_b", "path_c", "path_d"] {
        repo.run(&[
            "save",
            "--name",
            name,
            "--content",
            &format!("Action: use {name} in weighted graph traversal."),
            "--force",
        ]);
    }
    let payload = repo.join("weighted-edges.json");
    fs::write(
        &payload,
        r#"{"schema_version":1,"edges":[
          {"source":"path_a","target":"path_b","relation":"risk_for","confidence":"EXTRACTED","evidence":"A has a strong first hop to B."},
          {"source":"path_b","target":"path_d","relation":"related_to","confidence":"EXTRACTED","evidence":"B has a weak second hop to D."},
          {"source":"path_a","target":"path_c","relation":"depends_on","confidence":"EXTRACTED","evidence":"A depends on C."},
          {"source":"path_c","target":"path_d","relation":"depends_on","confidence":"EXTRACTED","evidence":"C depends on D."}
        ]}"#,
    )
    .expect("write weighted graph payload");
    repo.run(&["graph", "ingest", payload.to_str().expect("payload")]);

    let outgoing = json(&repo.run(&[
        "graph",
        "path",
        "path_a",
        "path_d",
        "--direction",
        "outgoing",
    ]));
    assert_eq!(outgoing["status"], "ok");
    assert_eq!(outgoing["hops"], 2);
    let labels = outgoing["nodes"]
        .as_array()
        .expect("path nodes")
        .iter()
        .filter_map(|node| node["label"].as_str())
        .collect::<Vec<_>>();
    assert!(labels.contains(&"path_c"), "weighted path: {outgoing}");
    assert!(!labels.contains(&"path_b"), "weighted path: {outgoing}");

    let wrong_direction = json(&repo.run(&[
        "graph",
        "path",
        "path_d",
        "path_a",
        "--direction",
        "outgoing",
    ]));
    assert_eq!(wrong_direction["status"], "not_found");
    let incoming = json(&repo.run(&[
        "graph",
        "path",
        "path_d",
        "path_a",
        "--direction",
        "incoming",
    ]));
    assert_eq!(incoming["status"], "ok");
    let any = json(&repo.run(&["graph", "path", "path_d", "path_a", "--direction", "any"]));
    assert_eq!(any["status"], "ok");
}

#[test]
fn graph_path_uses_stable_identity_for_equal_score_ties() {
    let repo = TestRepo::new("graph-stable-path-tie");
    repo.run(&["init"]);
    for name in ["tie_a", "tie_b", "tie_c", "tie_d"] {
        repo.run(&[
            "save",
            "--name",
            name,
            "--content",
            &format!("Action: use {name} in a deterministic path tie."),
            "--force",
        ]);
    }
    let payload = repo.join("equal-edges.json");
    fs::write(
        &payload,
        r#"{"schema_version":1,"edges":[
          {"source":"tie_a","target":"tie_b","relation":"related_to","confidence":"EXTRACTED","evidence":"Equal path through B, first hop."},
          {"source":"tie_b","target":"tie_d","relation":"related_to","confidence":"EXTRACTED","evidence":"Equal path through B, second hop."},
          {"source":"tie_a","target":"tie_c","relation":"related_to","confidence":"EXTRACTED","evidence":"Equal path through C, first hop."},
          {"source":"tie_c","target":"tie_d","relation":"related_to","confidence":"EXTRACTED","evidence":"Equal path through C, second hop."}
        ]}"#,
    )
    .expect("write equal-score graph payload");
    repo.run(&["graph", "ingest", payload.to_str().expect("payload")]);

    let conn = Connection::open(repo.join("memory.db")).expect("open graph db");
    for (source, target, edge_id) in [
        ("tie_a", "tie_b", "z-tie-a-b"),
        ("tie_b", "tie_d", "z-tie-b-d"),
        ("tie_a", "tie_c", "a-tie-a-c"),
        ("tie_c", "tie_d", "a-tie-c-d"),
    ] {
        conn.execute(
            "UPDATE graph_edges SET id = ?1
             WHERE source_node_id = (SELECT id FROM graph_nodes WHERE label = ?2)
               AND target_node_id = (SELECT id FROM graph_nodes WHERE label = ?3)
               AND relation = 'related_to'",
            rusqlite::params![edge_id, source, target],
        )
        .expect("set deterministic edge id");
    }
    drop(conn);

    let first = json(&repo.run(&["graph", "path", "tie_a", "tie_d", "--direction", "outgoing"]));
    let second = json(&repo.run(&["graph", "path", "tie_a", "tie_d", "--direction", "outgoing"]));
    assert_eq!(first, second);
    assert_eq!(first["hops"], 2);
    let labels = first["nodes"]
        .as_array()
        .expect("path nodes")
        .iter()
        .filter_map(|node| node["label"].as_str())
        .collect::<Vec<_>>();
    assert_eq!(labels, vec!["tie_a", "tie_c", "tie_d"]);
    let edge_ids = first["edges"]
        .as_array()
        .expect("path edges")
        .iter()
        .filter_map(|edge| edge["id"].as_str())
        .collect::<Vec<_>>();
    assert_eq!(edge_ids, vec!["a-tie-a-c", "a-tie-c-d"]);
}

#[test]
fn graph_reads_recover_missing_materialized_tables() {
    let repo = TestRepo::new("graph-materialization-recovery");
    repo.run(&["init"]);
    repo.run(&[
        "save",
        "--name",
        "recover_a",
        "--content",
        "Action: connect to the recovery target.",
        "--force",
    ]);
    repo.run(&[
        "save",
        "--name",
        "recover_b",
        "--content",
        "Action: receive the recovery source edge.",
        "--force",
    ]);
    let payload = repo.join("recovery-edge.json");
    fs::write(
        &payload,
        r#"{"schema_version":1,"edges":[{"source":"recover_a","target":"recover_b","relation":"depends_on","confidence":"EXTRACTED","evidence":"A explicitly depends on B."}]}"#,
    )
    .expect("write recovery edge");
    repo.run(&["graph", "ingest", payload.to_str().expect("payload")]);

    let conn = Connection::open(repo.join("memory.db")).expect("open graph db");
    conn.execute_batch("DROP TABLE graph_edges; DROP TABLE graph_nodes;")
        .expect("drop materialized graph");
    drop(conn);

    let recovered = json(&repo.run(&["graph", "path", "recover_a", "recover_b"]));
    assert_eq!(recovered["status"], "ok");
    let conn = Connection::open(repo.join("memory.db")).expect("reopen graph db");
    let edges: i64 = conn
        .query_row("SELECT COUNT(*) FROM graph_edges", [], |row| row.get(0))
        .expect("recovered graph edges");
    assert!(edges > 0);
}

#[test]
fn graph_query_expands_lexical_start_nodes() {
    let repo = TestRepo::new("graph-query");
    repo.run(&["init"]);
    save_reference(
        &repo,
        "release_policy",
        r#"["domain:release","risk:high"]"#,
        "Action: release requires checking `docs/release.md`.",
    );

    let output = json(&repo.run(&[
        "graph", "query", "release", "--scope", "all", "--depth", "1",
    ]));
    assert_eq!(output["status"], "ok");
    assert!(output["nodes"]
        .as_array()
        .expect("query nodes")
        .iter()
        .any(|node| node["node"]["id"] == "memory:release_policy"));
}

#[test]
fn graph_traversal_rejects_depth_and_result_limits_above_contract_bounds() {
    let repo = TestRepo::new("graph-query-bounds");
    repo.run(&["init"]);

    let query_depth = repo.run_fail(&["graph", "query", "anything", "--depth", "9"]);
    assert!(query_depth.contains("graph query depth must be between 0 and 8"));
    let query_limit = repo.run_fail(&["graph", "query", "anything", "--limit", "501"]);
    assert!(query_limit.contains("graph limit must be between 1 and 500"));
    let path_depth = repo.run_fail(&["graph", "path", "from", "to", "--max-depth", "21"]);
    assert!(path_depth.contains("graph path max depth must be between 1 and 20"));
}

#[test]
fn prime_focus_includes_budgeted_graph_context() {
    let repo = TestRepo::new("prime-focus");
    repo.run(&["init"]);
    save_reference(
        &repo,
        "release_policy",
        r#"["domain:release","risk:high"]"#,
        "Action: release requires checking `docs/release.md`.",
    );

    let home = repo.path().to_string_lossy().to_string();
    let output = json(&repo.run(&[
        "--home",
        &home,
        "prime",
        "--focus",
        "release",
        "--format",
        "json",
        "--per-section",
        "3",
    ]));
    assert_eq!(output["status"], "ok");
    assert!(output["graph_context"]["nodes"]
        .as_array()
        .expect("graph context nodes")
        .iter()
        .any(|node| node["node"]["id"] == "memory:release_policy"));

    let text = repo.run(&[
        "--home",
        &home,
        "prime",
        "--focus",
        "release",
        "--budget",
        "4000",
        "--per-section",
        "3",
    ]);
    assert!(text.contains("[graph focus]"));
    assert!(text.contains("release_policy"));
}

#[test]
fn graph_ingest_review_accept_reject_semantic_edges() {
    let repo = TestRepo::new("graph-semantic");
    repo.run(&["init"]);
    save_reference(
        &repo,
        "assistant_style",
        r#"["style:output"]"#,
        "Action: keep assistant output concise.",
    );
    save_reference(
        &repo,
        "release_policy",
        r#"["domain:release"]"#,
        "Action: ask before publishing a release.",
    );

    let payload_path = repo.join("semantic_edges.json");
    fs::write(
        &payload_path,
        r#"{
  "schema_version": 1,
  "edges": [
    {
      "source": "assistant_style",
      "target": "concept:assistant_output_style",
      "relation": "mentions_concept",
      "confidence": "EXTRACTED",
      "evidence": "The memory says to keep assistant output concise.",
      "source_spans": [{"memory": "assistant_style", "quote": "concise"}],
      "tags": ["style:output"]
    },
    {
      "source": "release_policy",
      "target": "assistant_style",
      "relation": "related_to",
      "confidence": "AMBIGUOUS",
      "evidence": "Both affect assistant behavior, but the link is uncertain."
    },
    {
      "source": "release_policy",
      "target": "assistant_style",
      "relation": "not_allowed",
      "confidence": "EXTRACTED",
      "evidence": "bad"
    }
  ]
}
"#,
    )
    .expect("write semantic payload");

    let ingested = json(&repo.run(&[
        "graph",
        "ingest",
        payload_path.to_str().expect("payload path"),
    ]));
    assert_eq!(ingested["inserted"], 2);
    assert_eq!(ingested["rejected"], 1);
    assert_eq!(ingested["pending"], 1);

    let active_path = json(&repo.run(&[
        "graph",
        "path",
        "assistant_style",
        "concept:assistant_output_style",
    ]));
    assert_eq!(active_path["status"], "ok");
    assert_eq!(active_path["hops"], 1);
    assert_eq!(active_path["edges"][0]["relation"], "mentions_concept");

    let review = json(&repo.run(&["graph", "review", "--pending"]));
    let pending = review["edges"].as_array().expect("pending edges");
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0]["confidence"], "AMBIGUOUS");
    let pending_id = pending[0]["id"].as_str().expect("pending id").to_string();
    let ambiguity_id = pending[0]["ambiguity_id"]
        .as_i64()
        .expect("linked ambiguity id");

    let pending_path = json(&repo.run(&[
        "graph",
        "path",
        "release_policy",
        "assistant_style",
        "--include-ambiguous",
    ]));
    assert_eq!(pending_path["status"], "ok");

    let accepted = json(&repo.run(&["graph", "accept", &pending_id]));
    assert_eq!(accepted["edge_status"], "active");
    let accepted_path = json(&repo.run(&["graph", "path", "release_policy", "assistant_style"]));
    assert_eq!(accepted_path["status"], "ok");
    let conn = Connection::open(repo.join("memory.db")).expect("open db");
    let resolution: String = conn
        .query_row(
            "SELECT resolution FROM ambiguities WHERE id = ?1",
            [ambiguity_id],
            |row| row.get(0),
        )
        .expect("ambiguity resolution");
    assert_ne!(resolution, "pending");

    let rejected = json(&repo.run(&["graph", "reject", &pending_id, "--note", "not useful"]));
    assert_eq!(rejected["edge_status"], "rejected");
    let review_all = json(&repo.run(&["graph", "review", "--ambiguous"]));
    assert_eq!(review_all["edges"][0]["status"], "rejected");
    let revisions: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM graph_semantic_edge_revisions WHERE edge_id = ?1",
            [&pending_id],
            |row| row.get(0),
        )
        .expect("semantic revisions");
    assert_eq!(revisions, 3);
}

#[test]
fn semantic_logical_identity_enforces_trust_and_expiry() {
    let repo = TestRepo::new("graph-semantic-trust");
    repo.run(&["init"]);
    save_reference(&repo, "trusted_policy", "[]", "Action: require approval.");
    repo.run(&[
        "save",
        "--name",
        "release_workflow_note",
        "--content",
        "Action: publish releases carefully.",
        "--force",
    ]);
    let manual_payload = repo.join("manual_edge.json");
    fs::write(
        &manual_payload,
        r#"{"schema_version":1,"edges":[{"source":"trusted_policy","target":"release_workflow_note","relation":"policy_for","confidence":"EXTRACTED","evidence":"The manual policy explicitly requires approval."}]}"#,
    )
    .expect("manual payload");
    let manual = json(&repo.run(&[
        "graph",
        "ingest",
        manual_payload.to_str().expect("manual payload"),
        "--source",
        "manual",
        "--user-confirmed",
    ]));
    assert_eq!(manual["inserted"], 1);

    let agent_payload = repo.join("agent_edge.json");
    fs::write(
        &agent_payload,
        r#"{"schema_version":1,"edges":[{"source":"trusted_policy","target":"release_workflow_note","relation":"policy_for","confidence":"INFERRED","evidence":"An agent supplied weaker alternative evidence."}]}"#,
    )
    .expect("agent payload");
    let agent = json(&repo.run(&[
        "graph",
        "ingest",
        agent_payload.to_str().expect("agent payload"),
    ]));
    assert_eq!(agent["rejected"], 1);
    assert_eq!(
        agent["results"][0]["reason"],
        "lower_trust_source_cannot_override_logical_edge"
    );

    let edge_id = manual["results"][0]["id"].as_str().expect("edge id");
    let conn = Connection::open(repo.join("memory.db")).expect("open db");
    let confirmed_at: Option<String> = conn
        .query_row(
            "SELECT user_confirmed_at FROM graph_semantic_edges WHERE id = ?1",
            [edge_id],
            |row| row.get(0),
        )
        .expect("semantic confirmation provenance");
    assert!(confirmed_at.is_some());
    conn.execute(
        "UPDATE graph_semantic_edges SET valid_until = '2000-01-01T00:00:00Z' WHERE id = ?1",
        [edge_id],
    )
    .expect("expire edge");
    conn.execute(
        "UPDATE metadata SET value = 'true' WHERE key = 'graph_dirty'",
        [],
    )
    .expect("mark dirty");
    drop(conn);
    let path = json(&repo.run(&["graph", "path", "trusted_policy", "release_workflow_note"]));
    assert_eq!(path["status"], "not_found");
}

#[test]
fn graph_path_excludes_administrative_metadata_bridges_by_default() {
    let repo = TestRepo::new("graph-metadata-bridges");
    repo.run(&["init"]);
    save_reference(&repo, "alpha_policy", "[]", "Action: keep alpha stable.");
    repo.run(&[
        "save",
        "--name",
        "beta_policy",
        "--content",
        "Action: document beta deployment ownership.",
        "--force",
    ]);

    let default_path = json(&repo.run(&[
        "graph",
        "path",
        "alpha_policy",
        "beta_policy",
        "--max-depth",
        "2",
    ]));
    assert_eq!(default_path["status"], "not_found");

    let metadata_path = json(&repo.run(&[
        "graph",
        "path",
        "alpha_policy",
        "beta_policy",
        "--max-depth",
        "2",
        "--include-metadata",
    ]));
    assert_eq!(metadata_path["status"], "ok");
}

#[test]
fn graph_rebuild_excludes_deleted_memories_but_preserves_supersession_lineage() {
    let repo = TestRepo::new("graph-lifecycle");
    repo.run(&["init"]);
    save_reference(&repo, "obsolete_note", "[]", "Action: use the old path.");
    repo.run(&[
        "supersede",
        "obsolete_note",
        "current_note",
        "--content",
        "Action: use the current path.",
    ]);
    save_reference(&repo, "temporary_note", "[]", "Action: temporary only.");
    repo.run(&["delete", "temporary_note"]);

    let lineage = json(&repo.run(&["graph", "path", "obsolete_note", "current_note"]));
    assert_eq!(lineage["status"], "ok");
    assert_eq!(lineage["edges"][0]["relation"], "superseded_by");

    let error = repo.run_fail(&["graph", "explain", "temporary_note"]);
    assert!(error.contains("node not found") || error.contains("memory not found"));
}

#[test]
fn cross_project_semantic_edges_require_review_for_agent_sources() {
    let repo = TestRepo::new("graph-cross-scope");
    repo.run(&["init"]);
    repo.run(&[
        "save",
        "--name",
        "project_a_policy",
        "--scope",
        "project:example/a",
        "--content",
        "Action: protect project A.",
    ]);
    repo.run(&[
        "save",
        "--name",
        "project_b_policy",
        "--scope",
        "project:example/b",
        "--content",
        "Action: protect project B.",
        "--force",
    ]);
    let payload = repo.join("cross_scope_edges.json");
    fs::write(
        &payload,
        r#"{"schema_version":1,"edges":[{"source":"project_a_policy","target":"project_b_policy","relation":"related_to","confidence":"EXTRACTED","evidence":"The projects share an explicit migration dependency."}]}"#,
    )
    .expect("write payload");

    let agent = json(&repo.run(&["graph", "ingest", payload.to_str().expect("payload")]));
    assert_eq!(agent["pending"], 1);
    assert_eq!(agent["results"][0]["edge_status"], "pending");
    let pending_path = json(&repo.run(&[
        "graph",
        "path",
        "project_a_policy",
        "project_b_policy",
        "--scope",
        "all",
    ]));
    assert_eq!(pending_path["status"], "not_found");
    let edge_id = agent["results"][0]["id"].as_str().expect("edge id");
    repo.run(&["graph", "accept", edge_id]);

    let scoped_error = repo.run_fail(&[
        "graph",
        "path",
        "project_a_policy",
        "project_b_policy",
        "--scope",
        "project:example/a",
    ]);
    assert!(scoped_error.contains("outside the selected scope"));
    let all_scopes = json(&repo.run(&[
        "graph",
        "path",
        "project_a_policy",
        "project_b_policy",
        "--scope",
        "all",
    ]));
    assert_eq!(all_scopes["status"], "ok");
}

#[test]
fn graph_stats_reports_dirty_without_implicitly_rebuilding() {
    let repo = TestRepo::new("graph-stats-dirty");
    repo.run(&["init"]);
    let initial = json(&repo.run(&["graph", "stats"]));
    assert_eq!(initial["dirty"], true);
    assert_eq!(initial["nodes"], 0);

    repo.run(&["graph", "rebuild"]);
    let clean = json(&repo.run(&["graph", "stats"]));
    assert_eq!(clean["dirty"], false);
    save_reference(&repo, "new_graph_input", "[]", "Action: mark graph dirty.");
    let dirty = json(&repo.run(&["graph", "stats"]));
    assert_eq!(dirty["dirty"], true);
}

#[test]
fn graph_candidates_support_changed_since_and_unlinked_filters() {
    let repo = TestRepo::new("graph-candidate-filters");
    repo.run(&["init"]);
    save_reference(&repo, "unlinked_memory", "[]", "Action: isolated memory.");
    save_reference(
        &repo,
        "tagged_memory",
        r#"["domain:linked"]"#,
        "Action: linked through a tag.",
    );

    let unlinked = json(&repo.run(&["graph", "candidates", "--scope", "all", "--unlinked"]));
    let names = unlinked["memories"]
        .as_array()
        .expect("candidate memories")
        .iter()
        .filter_map(|memory| memory["name"].as_str())
        .collect::<Vec<_>>();
    assert!(names.contains(&"unlinked_memory"));
    assert!(!names.contains(&"tagged_memory"));

    let future = json(&repo.run(&[
        "graph",
        "candidates",
        "--scope",
        "all",
        "--changed-since",
        "9999-01-01T00:00:00Z",
    ]));
    assert!(future["memories"].as_array().expect("memories").is_empty());
}
