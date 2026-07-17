use std::fs;
use std::process::Command;

mod support;

use support::{mem_bin, temp_path, TestRepo};

#[test]
fn contract_is_store_independent_and_versions_machine_interfaces() {
    let repo = TestRepo::new("machine-contract");

    let output: serde_json::Value =
        serde_json::from_str(&repo.run(&["contract"])).expect("contract json");

    assert_eq!(output["status"], "ok");
    assert_eq!(output["contract_version"], 1);
    assert_eq!(output["cli_version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(output["json_errors"]["version"], 1);
    assert_eq!(output["schemas"]["store"], 5);
    assert_eq!(output["schemas"]["bundle"], 2);
    assert_eq!(output["schemas"]["workflow"], 1);
    assert_eq!(output["schemas"]["graph"], 1);
    assert_eq!(output["schemas"]["benchmark_report"], 1);
    assert!(!repo.join("memory.db").exists());
}

#[test]
fn contract_ignores_malformed_runtime_configuration() {
    let config_root = temp_path("contract-malformed-config");
    let config_dir = config_root.join("mnemark");
    fs::create_dir_all(&config_dir).expect("config dir");
    fs::write(config_dir.join("config.toml"), "not valid = [toml").expect("malformed config");

    let output = Command::new(mem_bin())
        .env("XDG_CONFIG_HOME", &config_root)
        .arg("contract")
        .output()
        .expect("run contract");

    assert!(
        output.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let contract: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("contract json");
    assert_eq!(contract["contract_version"], 1);
    fs::remove_dir_all(config_root).ok();
}

#[test]
fn core_success_json_keeps_required_v1_fields() {
    let repo = TestRepo::new("success-contract");
    let initialized: serde_json::Value =
        serde_json::from_str(&repo.run(&["init"])).expect("init json");
    assert_eq!(initialized["status"], "initialized");
    assert!(initialized["root"].is_string());

    let saved: serde_json::Value = serde_json::from_str(&repo.run(&[
        "save",
        "--name",
        "contract_memory",
        "--content",
        "machine-readable contract payload",
        "--force",
    ]))
    .expect("save json");
    assert_eq!(saved["status"], "saved");
    assert!(saved["id"].is_string());
    assert_eq!(saved["version"], 1);

    let queried: serde_json::Value =
        serde_json::from_str(&repo.run(&["query", "machine-readable contract"]))
            .expect("query json");
    let memory = queried
        .as_array()
        .and_then(|rows| rows.first())
        .expect("query row");
    for field in [
        "id",
        "type",
        "name",
        "tags",
        "scope",
        "source",
        "confidence",
        "version",
    ] {
        assert!(memory.get(field).is_some(), "missing query field {field}");
    }

    let config: serde_json::Value =
        serde_json::from_str(&repo.run(&["config", "show"])).expect("config json");
    for field in ["root", "store_source", "db_path", "index_path", "effective"] {
        assert!(config.get(field).is_some(), "missing config field {field}");
    }
}
