use std::fs;
use std::process::Command;

mod support;

use support::{mem_bin, temp_path, TestRepo, TestRuntimeStore};

#[test]
fn source_checkout_is_never_selected_implicitly() {
    let checkout = TestRepo::new("store-warning-checkout");
    let runtime = temp_path("store-warning-selected-runtime");
    let init = Command::new(mem_bin())
        .current_dir(checkout.path())
        .env("MNEMARK_HOME", &runtime)
        .arg("init")
        .output()
        .expect("init runtime store");
    assert!(
        init.status.success(),
        "{}",
        String::from_utf8_lossy(&init.stderr)
    );

    let output = Command::new(mem_bin())
        .current_dir(checkout.path())
        .env("MNEMARK_HOME", &runtime)
        .args([
            "save",
            "--name",
            "runtime_fact_from_checkout",
            "--tags",
            "[\"domain:test\"]",
            "--content",
            "Action: use the runtime store. Why: 2026-07-09 test.",
        ])
        .output()
        .expect("save runtime memory");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let result: serde_json::Value = serde_json::from_slice(&output.stdout).expect("save json");
    assert_eq!(result["status"], "saved");
    assert!(runtime.join("memory.db").exists());
    assert!(!checkout.join("memory.db").exists());
    assert!(result.get("store_warning").is_none(), "result: {result}");
    fs::remove_dir_all(runtime).ok();
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
