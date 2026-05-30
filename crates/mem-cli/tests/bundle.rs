use std::fs;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

mod support;

use support::TestRepo;

#[test]
fn bundle_export_inspect_and_import_clean_store() {
    let source = TestRepo::new("bundle-source");
    source.run(&["init"]);
    source.run(&[
        "save",
        "--name",
        "bundle_memory",
        "--content",
        "portable bundle memory payload",
        "--force",
    ]);
    write_file(
        source.join("artifacts/scripts/ci-triage.sh"),
        "hello\n",
        true,
    );
    source.run(&[
        "artifact",
        "add",
        "artifacts/scripts/ci-triage.sh",
        "--kind",
        "script",
        "--scope",
        "global",
        "--executable",
    ]);
    fs::write(source.join("index/should-not-export"), "ignored").expect("write ignored index");
    fs::write(source.join("config.toml"), "[query]\ndefault_limit = 3\n").expect("write config");

    let bundle = source.join("store.tgz");
    let exported = source.run(&["bundle", "export", bundle.to_str().expect("bundle path")]);
    assert!(exported.contains(r#""status": "exported""#));

    let inspected: serde_json::Value = serde_json::from_str(&source.run(&[
        "bundle",
        "inspect",
        bundle.to_str().expect("bundle path"),
    ]))
    .expect("inspect json");
    let entries = inspected["entries"].as_array().expect("entries");
    assert!(entries.iter().any(|entry| entry == "memory.db"));
    assert!(entries.iter().any(|entry| entry == "manifest.toml"));
    assert!(entries.iter().any(|entry| entry == "config.toml"));
    assert!(entries.iter().any(|entry| entry == "bundle.json"));
    assert!(!entries
        .iter()
        .filter_map(serde_json::Value::as_str)
        .any(|entry| entry.starts_with("index/")));

    let target = TestRepo::new("bundle-target");
    let imported = target.run(&["bundle", "import", bundle.to_str().expect("bundle path")]);
    assert!(imported.contains(r#""status": "imported""#));
    assert!(imported.contains(r#""mode": "clean""#));

    let query = target.run(&["query", "portable bundle", "--no-touch"]);
    assert!(query.contains("bundle_memory"));
    let checked: serde_json::Value =
        serde_json::from_str(&target.run(&["artifact", "check"])).expect("check json");
    assert_eq!(checked["status"], "ok");
}

#[test]
fn bundle_export_can_omit_config() {
    let source = TestRepo::new("bundle-no-config-source");
    source.run(&["init"]);
    fs::write(source.join("config.toml"), "[query]\ndefault_limit = 3\n").expect("write config");

    let bundle = source.join("store-no-config.tgz");
    let exported = source.run(&[
        "bundle",
        "export",
        bundle.to_str().expect("bundle path"),
        "--no-config",
    ]);
    assert!(exported.contains(r#""config": false"#));

    let inspected: serde_json::Value = serde_json::from_str(&source.run(&[
        "bundle",
        "inspect",
        bundle.to_str().expect("bundle path"),
    ]))
    .expect("inspect json");
    assert!(!inspected["entries"]
        .as_array()
        .expect("entries")
        .iter()
        .any(|entry| entry == "config.toml"));
}

#[test]
fn bundle_import_refuses_non_empty_store_unless_replace_is_forced() {
    let source = TestRepo::new("bundle-replace-source");
    source.run(&["init"]);
    source.run(&[
        "save",
        "--name",
        "incoming_bundle_memory",
        "--content",
        "incoming bundle payload",
        "--force",
    ]);
    let bundle = source.join("store.tgz");
    source.run(&["bundle", "export", bundle.to_str().expect("bundle path")]);

    let target = TestRepo::new("bundle-replace-target");
    target.run(&["init"]);
    target.run(&[
        "save",
        "--name",
        "local_only_memory",
        "--content",
        "local only payload",
        "--force",
    ]);

    let refused = target.run_fail(&["bundle", "import", bundle.to_str().expect("bundle path")]);
    assert!(refused.contains("active store is not empty"));
    let replace_without_force = target.run_fail(&[
        "bundle",
        "import",
        bundle.to_str().expect("bundle path"),
        "--replace",
    ]);
    assert!(replace_without_force.contains("--replace requires --force"));

    let replaced = target.run(&[
        "bundle",
        "import",
        bundle.to_str().expect("bundle path"),
        "--replace",
        "--force",
    ]);
    assert!(replaced.contains(r#""mode": "clean""#));
    let incoming = target.run(&["query", "incoming bundle", "--no-touch"]);
    assert!(incoming.contains("incoming_bundle_memory"));
    let local = target.run(&["query", "local only", "--no-touch"]);
    assert!(!local.contains("local_only_memory"));
}

#[test]
fn bundle_import_merge_keeps_local_store_and_adds_bundle_contents() {
    let source = TestRepo::new("bundle-merge-source");
    source.run(&["init"]);
    source.run(&[
        "save",
        "--name",
        "incoming_merge_memory",
        "--content",
        "incoming merge payload",
        "--force",
    ]);
    write_file(
        source.join("artifacts/scripts/merge-helper.sh"),
        "hello\n",
        true,
    );
    source.run(&[
        "artifact",
        "add",
        "artifacts/scripts/merge-helper.sh",
        "--kind",
        "script",
        "--scope",
        "global",
        "--executable",
    ]);
    let bundle = source.join("store.tgz");
    source.run(&["bundle", "export", bundle.to_str().expect("bundle path")]);

    let target = TestRepo::new("bundle-merge-target");
    target.run(&["init"]);
    target.run(&[
        "save",
        "--name",
        "local_merge_memory",
        "--content",
        "local merge payload",
        "--force",
    ]);

    let merged = target.run(&[
        "bundle",
        "import",
        bundle.to_str().expect("bundle path"),
        "--merge",
    ]);
    assert!(merged.contains(r#""mode": "merge""#));
    assert!(merged.contains(r#""imported": 1"#));
    assert!(target
        .run(&["query", "local merge", "--no-touch"])
        .contains("local_merge_memory"));
    assert!(target
        .run(&["query", "incoming merge", "--no-touch"])
        .contains("incoming_merge_memory"));
    let checked: serde_json::Value =
        serde_json::from_str(&target.run(&["artifact", "check"])).expect("check json");
    assert_eq!(checked["status"], "ok");
}

fn write_file(path: std::path::PathBuf, content: &str, executable: bool) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("artifact dir");
    }
    fs::write(&path, content).expect("write artifact");
    #[cfg(unix)]
    if executable {
        let mut permissions = fs::metadata(&path).expect("metadata").permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&path, permissions).expect("chmod artifact");
    }
}
