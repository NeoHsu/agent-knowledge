use std::fs;
use std::process::Command;

mod support;

use support::{TestRepo, mem_bin, temp_path};

fn schema_root() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../docs/schemas")
}

fn load_schema(name: &str) -> serde_json::Value {
    serde_json::from_slice(
        &fs::read(schema_root().join(format!("{name}.schema.json")))
            .unwrap_or_else(|error| panic!("read schema {name}: {error}")),
    )
    .unwrap_or_else(|error| panic!("parse schema {name}: {error}"))
}

fn validate_schema_fixture(schema: &serde_json::Value, value: &serde_json::Value, path: &str) {
    if let Some(expected) = schema.get("const") {
        assert_eq!(value, expected, "{path}: const mismatch");
    }
    if let Some(values) = schema.get("enum").and_then(serde_json::Value::as_array) {
        assert!(values.contains(value), "{path}: value is not in enum");
    }
    if let Some(types) = schema.get("type") {
        let matches_type = |kind: &str| match kind {
            "object" => value.is_object(),
            "array" => value.is_array(),
            "string" => value.is_string(),
            "integer" => value.as_i64().is_some() || value.as_u64().is_some(),
            "boolean" => value.is_boolean(),
            "null" => value.is_null(),
            _ => true,
        };
        let valid = types.as_str().is_some_and(matches_type)
            || types.as_array().is_some_and(|types| {
                types
                    .iter()
                    .filter_map(serde_json::Value::as_str)
                    .any(matches_type)
            });
        assert!(valid, "{path}: JSON type does not match schema");
    }
    if let Some(required) = schema.get("required").and_then(serde_json::Value::as_array) {
        let object = value.as_object().expect("required applies to an object");
        for key in required.iter().filter_map(serde_json::Value::as_str) {
            assert!(object.contains_key(key), "{path}: missing required `{key}`");
        }
    }
    if let (Some(properties), Some(object)) = (
        schema
            .get("properties")
            .and_then(serde_json::Value::as_object),
        value.as_object(),
    ) {
        for (key, property_schema) in properties {
            if let Some(property) = object.get(key) {
                validate_schema_fixture(property_schema, property, &format!("{path}.{key}"));
            }
        }
        match schema.get("additionalProperties") {
            Some(serde_json::Value::Bool(false)) => {
                for key in object.keys() {
                    assert!(
                        properties.contains_key(key),
                        "{path}: unknown property `{key}`"
                    );
                }
            }
            Some(additional_schema @ serde_json::Value::Object(_)) => {
                for (key, property) in object {
                    if !properties.contains_key(key) {
                        validate_schema_fixture(
                            additional_schema,
                            property,
                            &format!("{path}.{key}"),
                        );
                    }
                }
            }
            _ => {}
        }
    }
    if let (Some(item_schema), Some(items)) = (schema.get("items"), value.as_array()) {
        for (index, item) in items.iter().enumerate() {
            validate_schema_fixture(item_schema, item, &format!("{path}[{index}]"));
        }
    }
    if let (Some(min_items), Some(items)) = (
        schema.get("minItems").and_then(serde_json::Value::as_u64),
        value.as_array(),
    ) {
        assert!(items.len() as u64 >= min_items, "{path}: too few items");
    }
    if let (Some(min_length), Some(text)) = (
        schema.get("minLength").and_then(serde_json::Value::as_u64),
        value.as_str(),
    ) {
        assert!(
            text.chars().count() as u64 >= min_length,
            "{path}: string is too short"
        );
    }
}

#[test]
fn every_published_schema_has_a_valid_fixture() {
    let root = schema_root();
    let fixtures = root.join("fixtures");
    let mut names = Vec::new();
    for entry in fs::read_dir(&root).expect("read schema directory") {
        let path = entry.expect("schema entry").path();
        let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        let Some(name) = file_name.strip_suffix(".schema.json") else {
            continue;
        };
        let schema: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).expect("read schema")).expect("parse schema");
        let fixture_path = fixtures.join(format!("{name}.json"));
        let fixture: serde_json::Value = serde_json::from_slice(
            &fs::read(&fixture_path)
                .unwrap_or_else(|error| panic!("read {}: {error}", fixture_path.display())),
        )
        .expect("parse fixture");
        validate_schema_fixture(&schema, &fixture, name);
        names.push(name.to_string());
    }
    names.sort();
    assert_eq!(names.len(), 10, "public schema count changed unexpectedly");

    let compatibility: serde_json::Value = serde_json::from_slice(
        &fs::read(
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../skills/mnemark/compatibility.json"),
        )
        .expect("read skill compatibility"),
    )
    .expect("parse skill compatibility");
    let schema: serde_json::Value = serde_json::from_slice(
        &fs::read(root.join("skill-compatibility-v1.schema.json"))
            .expect("read compatibility schema"),
    )
    .expect("parse compatibility schema");
    validate_schema_fixture(&schema, &compatibility, "skill-compatibility-v1");
}

#[test]
fn schema_and_operation_discovery_are_store_independent() {
    let repo = TestRepo::new("discovery-contract");
    let schemas: serde_json::Value =
        serde_json::from_str(&repo.run(&["schema", "list"])).expect("schema list");
    assert_eq!(schemas["contract_version"], 1);
    assert_eq!(schemas["schemas"].as_array().expect("schemas").len(), 10);
    validate_schema_fixture(&load_schema("schema-list-v1"), &schemas, "schema-list-v1");

    let error_schema: serde_json::Value =
        serde_json::from_str(&repo.run(&["schema", "print", "error-v1"])).expect("error schema");
    assert_eq!(error_schema["title"], "mnemark JSON error v1");

    let operation_list: serde_json::Value =
        serde_json::from_str(&repo.run(&["operation", "list"])).expect("operation list");
    validate_schema_fixture(
        &load_schema("operation-list-v1"),
        &operation_list,
        "operation-list-v1",
    );
    let operations = operation_list["operations"].as_array().expect("operations");
    assert!(operations.len() > 40);
    assert!(
        operations
            .iter()
            .any(|operation| operation["id"] == "graph.query")
    );

    let inspected: serde_json::Value = serde_json::from_str(&repo.run(&[
        "operation",
        "inspect",
        "--store-exists",
        "--",
        "query",
        "release",
        "--touch",
    ]))
    .expect("operation inspect");
    validate_schema_fixture(
        &load_schema("operation-inspect-v1"),
        &inspected,
        "operation-inspect-v1",
    );
    assert_eq!(inspected["operation"], "query");
    assert_eq!(inspected["effect"]["store_access"], "exclusive_lock");
    assert_eq!(inspected["effect"]["durable_write"], true);
}

#[test]
fn public_schemas_accept_current_command_outputs() {
    let repo = TestRepo::new("public-schema-runtime");
    repo.run(&["init"]);
    repo.run(&[
        "save",
        "--name",
        "schema_contract_sample",
        "--description",
        "Representative runtime schema row",
        "--content",
        "Trigger: contract validation. Action: validate runtime JSON. Why: prevent schema drift.",
        "--tags",
        "[\"domain:contracts\"]",
        "--force",
    ]);

    let samples = [
        (
            "contract-v1",
            serde_json::from_str(&repo.run(&["contract"])).expect("contract output"),
        ),
        (
            "memory-list-v1",
            serde_json::from_str(&repo.run(&[
                "query",
                "schema_contract_sample",
                "--format",
                "json",
            ]))
            .expect("memory query output"),
        ),
        (
            "prime-v1",
            serde_json::from_str(&repo.run(&["prime", "--format", "json"])).expect("prime output"),
        ),
        (
            "graph-export-v1",
            serde_json::from_str(&repo.run(&["graph", "export", "--format", "json"]))
                .expect("graph export output"),
        ),
    ];
    for (name, output) in samples {
        validate_schema_fixture(&load_schema(name), &output, name);
    }

    let bundle_path = repo.join("schema-contract.tgz");
    let bundle_path_text = bundle_path.to_string_lossy().into_owned();
    let bundle_output: serde_json::Value =
        serde_json::from_str(&repo.run(&["bundle", "export", &bundle_path_text]))
            .expect("bundle export output");
    validate_schema_fixture(
        &load_schema("bundle-manifest-v2"),
        &bundle_output["bundle"],
        "bundle-manifest-v2",
    );

    let missing = TestRepo::new("public-schema-error");
    let error: serde_json::Value =
        serde_json::from_str(&missing.run_fail(&["--json-errors", "query", "missing"]))
            .expect("error output");
    validate_schema_fixture(&load_schema("error-v1"), &error, "error-v1");
}

#[test]
fn skill_version_gate_fails_before_store_discovery() {
    let config_root = temp_path("skill-gate-malformed-config");
    let config_dir = config_root.join("mnemark");
    fs::create_dir_all(&config_dir).expect("config directory");
    fs::write(config_dir.join("config.toml"), "not valid = [toml").expect("config");

    let output = Command::new(mem_bin())
        .env("XDG_CONFIG_HOME", &config_root)
        .args(["--json-errors", "contract", "--skill-version", "0.8.0"])
        .output()
        .expect("run skill gate");
    assert_eq!(output.status.code(), Some(2));
    let stdout: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("compatibility report stdout");
    let stderr: serde_json::Value =
        serde_json::from_slice(&output.stderr).expect("version error stderr");
    assert_eq!(stdout["skill_compatibility"]["compatible"], false);
    assert_eq!(stderr["code"], "version_mismatch");
    assert_eq!(stderr["retryable"], false);
    assert!(
        stderr["details"]["update_command"]
            .as_str()
            .expect("update command")
            .contains("tree/v0.9.0")
    );
    fs::remove_dir_all(config_root).ok();
}
