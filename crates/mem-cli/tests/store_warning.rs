mod support;

use support::{TestRepo, TestRuntimeStore};

#[test]
fn writes_into_a_source_checkout_carry_a_store_warning() {
    let repo = TestRepo::new("store-warning-checkout");
    repo.run(&["init"]);

    let output = repo.run(&[
        "save",
        "--name",
        "checkout_fact",
        "--tags",
        "[\"domain:test\"]",
        "--content",
        "Action: fact saved into a schema-discovered store. Why: 2026-07-09 test.",
    ]);
    let result: serde_json::Value = serde_json::from_str(&output).expect("save json");
    assert_eq!(result["status"], "saved");
    assert_eq!(result["store_source"], "current_directory");
    assert!(
        result["store_warning"]
            .as_str()
            .expect("store warning")
            .contains("--home"),
        "result: {result}"
    );

    let output = repo.run(&["update", "checkout_fact", "--content", "Action: updated."]);
    let result: serde_json::Value = serde_json::from_str(&output).expect("update json");
    assert_eq!(result["store_source"], "current_directory");

    let output = repo.run(&["delete", "checkout_fact"]);
    let result: serde_json::Value = serde_json::from_str(&output).expect("delete json");
    assert_eq!(result["store_source"], "current_directory");
}

#[test]
fn writes_into_the_runtime_store_carry_no_store_warning() {
    let store = TestRuntimeStore::new("store-warning-runtime");
    store.run(&["init"]);

    let output = store.run(&[
        "save",
        "--name",
        "runtime_fact",
        "--tags",
        "[\"domain:test\"]",
        "--content",
        "Action: fact saved into the runtime store. Why: 2026-07-09 test.",
    ]);
    let result: serde_json::Value = serde_json::from_str(&output).expect("save json");
    assert_eq!(result["status"], "saved");
    assert!(result.get("store_warning").is_none(), "result: {result}");
    assert!(result.get("store_source").is_none(), "result: {result}");
}
