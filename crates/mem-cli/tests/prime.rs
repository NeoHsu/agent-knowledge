use std::fs;

mod support;

use support::TestRuntimeStore;

#[test]
fn prime_renders_sections_and_protocol() {
    let store = TestRuntimeStore::new("prime-basic");
    store.run(&["init"]);
    store.run(&[
        "save",
        "--type",
        "feedback",
        "--name",
        "no_emoji",
        "--tags",
        "[\"style:no-emoji\"]",
        "--content",
        "產出面向使用者的文字時不要使用 emoji。原因：使用者要求。",
    ]);
    store.run(&[
        "save",
        "--type",
        "project",
        "--name",
        "release_owner",
        "--tags",
        "[\"project:example/app\"]",
        "--content",
        "release owner is alice",
    ]);

    let output = store.run(&["prime"]);
    assert!(output.contains("=== mnemark context"), "header: {output}");
    assert!(output.contains("[feedback]"), "feedback section: {output}");
    assert!(output.contains("- no_emoji ::"), "entry line: {output}");
    assert!(output.contains("[project]"), "project section: {output}");
    assert!(output.contains("-- protocol --"), "protocol: {output}");
    assert!(output.contains("mem save"), "protocol mentions save: {output}");
}

#[test]
fn prime_shows_workflow_goal_not_body() {
    let store = TestRuntimeStore::new("prime-workflow");
    store.run(&["init"]);
    let workflow = "schema_version: 1\ngoal: ship a release safely\ntriggers:\n  - release\nsteps:\n  - id: build\n    check: build passes\nstop_conditions:\n  - checks fail\n";
    let path = store.run_dir().join("wf.yaml");
    fs::write(&path, workflow).expect("write workflow");
    store.run(&[
        "save",
        "--type",
        "workflow",
        "--name",
        "release_runbook",
        "--tags",
        "[\"workflow:release\"]",
        "--content-file",
        path.to_str().expect("path"),
    ]);

    let output = store.run(&["prime"]);
    assert!(output.contains("[workflow]"), "workflow section: {output}");
    assert!(
        output.contains("- release_runbook :: ship a release safely"),
        "goal line: {output}"
    );
    assert!(
        !output.contains("schema_version"),
        "runbook body must not prime: {output}"
    );
}

#[test]
fn prime_respects_budget() {
    let store = TestRuntimeStore::new("prime-budget");
    store.run(&["init"]);
    let long = "x".repeat(500);
    for index in 0..10 {
        store.run(&[
            "save",
            "--type",
            "feedback",
            "--name",
            &format!("bulk_{index}"),
            "--tags",
            "[\"domain:test\"]",
            "--force",
            "--content",
            &format!("{index} {long}"),
        ]);
    }
    let output = store.run(&["prime", "--budget", "600"]);
    assert!(
        output.chars().count() <= 900,
        "budget overshoot: {} chars",
        output.chars().count()
    );
    assert!(output.contains("-- protocol --"));
}

#[test]
fn prime_without_store_reports_and_succeeds() {
    let store = TestRuntimeStore::new("prime-no-store");
    let output = store.run(&["prime"]);
    assert!(output.contains("no memory store"), "message: {output}");
    assert!(output.contains("mem init"), "hint: {output}");
}

#[test]
fn prime_ignores_source_checkout_in_cwd() {
    let store = TestRuntimeStore::new("prime-runtime-only");
    store.run(&["init"]);
    // Make the run directory look like a source checkout; prime must still
    // target the MNEMARK_HOME runtime store, not the cwd store.
    fs::create_dir_all(store.run_dir().join("schema")).expect("schema dir");
    fs::write(
        store.run_dir().join("schema/memory-schema.sql"),
        include_str!("../../../schema/memory-schema.sql"),
    )
    .expect("schema");

    let output = store.run(&["prime"]);
    let home = store.home().display().to_string();
    assert!(
        output.contains(&home),
        "prime must use runtime store {home}: {output}"
    );
    assert!(!store.run_dir().join("memory.db").exists());
}

#[test]
fn prime_json_format_parses() {
    let store = TestRuntimeStore::new("prime-json");
    store.run(&["init"]);
    store.run(&[
        "save",
        "--type",
        "preference",
        "--name",
        "reply_language",
        "--tags",
        "[\"style:language\"]",
        "--content",
        "回覆一律使用繁體中文",
    ]);
    let output = store.run(&["prime", "--format", "json"]);
    let parsed: serde_json::Value = serde_json::from_str(&output).expect("prime json");
    assert_eq!(parsed["status"], "ok");
    assert_eq!(parsed["sections"]["preference"][0]["name"], "reply_language");
}
