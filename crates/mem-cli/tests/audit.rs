use rusqlite::Connection;
use std::fs;

mod support;

use support::TestRepo;

fn save_reference(repo: &TestRepo, name: &str) {
    repo.run(&[
        "save",
        "--name",
        name,
        "--tags",
        "[\"domain:budget-test\"]",
        "--force",
        "--content",
        &format!("Action: fact number {name}. Why: 2026-07-08 test."),
    ]);
}

#[test]
fn audit_fix_increments_versions_for_every_memory_mutation() {
    let repo = TestRepo::new("audit-fix-versions");
    repo.run(&["init"]);
    repo.run(&[
        "save",
        "--name",
        "expired_version",
        "--tags",
        "[\"test:audit\"]",
        "--expires-at",
        "2020-01-01T00:00:00Z",
        "--content",
        "expired audit version probe",
        "--force",
    ]);
    repo.run(&[
        "save",
        "--name",
        "broken_link_version",
        "--tags",
        "[\"test:audit\"]",
        "--content",
        "broken-link audit version probe",
        "--force",
    ]);
    let conn = Connection::open(repo.join("memory.db")).expect("open memory db");
    conn.execute(
        "UPDATE memories SET superseded_by = 'missing-memory' WHERE name = 'broken_link_version'",
        [],
    )
    .expect("inject broken link");
    drop(conn);

    let report: serde_json::Value =
        serde_json::from_str(&repo.run(&["audit", "--fix"])).expect("audit report");
    assert_eq!(report["fixed_expired"], 1);
    assert_eq!(report["fixed_broken_links"], 1);

    let conn = Connection::open(repo.join("memory.db")).expect("open fixed memory db");
    let expired: (i64, Option<String>) = conn
        .query_row(
            "SELECT version, valid_until FROM memories WHERE name = 'expired_version'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("expired mutation");
    let broken: (i64, Option<String>) = conn
        .query_row(
            "SELECT version, superseded_by FROM memories WHERE name = 'broken_link_version'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("broken-link mutation");
    assert_eq!(expired.0, 2);
    assert!(expired.1.is_some());
    assert_eq!(broken, (2, None));
}

#[test]
fn audit_flags_scopes_over_the_configured_budget() {
    let repo = TestRepo::new("audit-budget");
    repo.run(&["init"]);
    fs::write(repo.join("config.toml"), "[budget]\nper_scope_max = 2\n").expect("write config");
    for name in ["budget_fact_one", "budget_fact_two", "budget_fact_three"] {
        save_reference(&repo, name);
    }

    let output = repo.run(&["audit"]);
    let report: serde_json::Value = serde_json::from_str(&output).expect("audit json");
    assert_eq!(report["per_scope_max"], 2);
    let over = report["over_budget_scopes"]
        .as_array()
        .expect("over_budget_scopes");
    assert_eq!(over.len(), 1, "report: {report}");
    assert_eq!(over[0]["scope"], "global");
    assert_eq!(over[0]["count"], 3);
    let candidates = over[0]["curation_candidates"]
        .as_array()
        .expect("candidates");
    assert_eq!(candidates.len(), 3);

    // The weekly retro bundle embeds the audit report, so curation pressure
    // reaches retros without a separate command.
    let output = repo.run(&["retro", "weekly"]);
    let bundle: serde_json::Value = serde_json::from_str(&output).expect("retro json");
    assert_eq!(
        bundle["audit"]["over_budget_scopes"][0]["scope"], "global",
        "bundle: {bundle}"
    );
}

#[test]
fn audit_budget_can_be_disabled_and_defaults_to_thirty() {
    let repo = TestRepo::new("audit-budget-off");
    repo.run(&["init"]);
    fs::write(repo.join("config.toml"), "[budget]\nper_scope_max = 0\n").expect("write config");
    for name in ["fact_a", "fact_b", "fact_c"] {
        save_reference(&repo, name);
    }

    let output = repo.run(&["audit"]);
    let report: serde_json::Value = serde_json::from_str(&output).expect("audit json");
    assert_eq!(report["per_scope_max"], 0);
    assert_eq!(report["over_budget_scopes"], serde_json::json!([]));

    fs::remove_file(repo.join("config.toml")).expect("remove config");
    let output = repo.run(&["config", "show"]);
    let shown: serde_json::Value = serde_json::from_str(&output).expect("config json");
    assert_eq!(shown["effective"]["budget_per_scope_max"], 30);
}

#[test]
fn audit_graph_health_marks_stale_derivations_and_reports_current_orphans() {
    let repo = TestRepo::new("audit-graph-health");
    repo.run(&["init"]);
    repo.run(&[
        "save",
        "--name",
        "isolated_memory",
        "--content",
        "Action: retain one isolated fact.",
        "--force",
    ]);

    let stale: serde_json::Value =
        serde_json::from_str(&repo.run(&["audit"])).expect("stale audit json");
    assert_eq!(stale["graph"]["derived_status"], "stale");
    assert!(stale["graph"]["orphan_memories"].is_null());

    repo.run(&["graph", "rebuild"]);
    let current: serde_json::Value =
        serde_json::from_str(&repo.run(&["audit"])).expect("current audit json");
    assert_eq!(current["graph"]["derived_status"], "current");
    assert!(
        current["graph"]["orphan_memories"]
            .as_array()
            .expect("orphan memories")
            .iter()
            .any(|memory| memory["id"] == "memory:isolated_memory")
    );
}

#[test]
fn audit_budget_candidates_exclude_protected_memories() {
    let repo = TestRepo::new("audit-budget-protected");
    repo.run(&["init"]);
    fs::write(repo.join("config.toml"), "[budget]\nper_scope_max = 1\n").expect("write config");
    repo.run(&[
        "save",
        "--name",
        "protected_fact",
        "--source",
        "manual",
        "--user-confirmed",
        "--tags",
        "[\"domain:budget-test\"]",
        "--content",
        "Action: protected fact. Why: 2026-07-08 test.",
    ]);
    save_reference(&repo, "agent_fact");

    let output = repo.run(&["audit"]);
    let report: serde_json::Value = serde_json::from_str(&output).expect("audit json");
    let candidates = report["over_budget_scopes"][0]["curation_candidates"]
        .as_array()
        .expect("candidates");
    assert_eq!(candidates.len(), 1, "report: {report}");
    assert_eq!(candidates[0]["name"], "agent_fact");
}
