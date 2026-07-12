use std::fs;
use std::fs::File;
use std::path::Path;

use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;
use tar::{Archive, Builder, EntryType, Header};

use mem_core::artifact::artifact_file_checksum;

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
    assert_eq!(inspected["bundle"]["version"], 2);
    assert!(inspected["bundle"]["hashes"]["memory.db"]
        .as_str()
        .is_some_and(|hash| hash.starts_with("sha256:")));
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
    let semantic_payload = source.join("semantic_edges.json");
    fs::write(
        &semantic_payload,
        r#"{"schema_version":1,"edges":[{"source":"incoming_merge_memory","target":"artifacts/scripts/merge-helper.sh","relation":"depends_on","confidence":"EXTRACTED","evidence":"The incoming memory explicitly depends on the merge helper artifact."}]}"#,
    )
    .expect("write semantic payload");
    source.run(&[
        "graph",
        "ingest",
        semantic_payload.to_str().expect("semantic payload"),
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
    let path: serde_json::Value = serde_json::from_str(&target.run(&[
        "graph",
        "path",
        "incoming_merge_memory",
        "artifact:artifacts/scripts/merge-helper.sh",
    ]))
    .expect("graph path json");
    assert_eq!(path["status"], "ok");
    assert_eq!(path["edges"][0]["relation"], "depends_on");
}

#[test]
fn bundle_export_rejects_secrets_or_redacts_only_the_exported_copy() {
    let source = TestRepo::new("bundle-secret-source");
    source.run(&["init"]);
    source.run(&[
        "save",
        "--name",
        "bundle_secret",
        "--content",
        "Action: create a safe placeholder.",
        "--force",
    ]);
    let secret = "ghp_abcdefghijklmnop1234567890";
    let conn = rusqlite::Connection::open(source.join("memory.db")).expect("open source db");
    conn.execute(
        "UPDATE memories SET description = ?1, content = ?1 WHERE name = 'bundle_secret'",
        [format!("leaked {secret}")],
    )
    .expect("inject pre-existing secret");
    drop(conn);

    let rejected_bundle = source.join("rejected.tgz");
    let error = source.run_fail(&[
        "bundle",
        "export",
        rejected_bundle.to_str().expect("rejected bundle"),
    ]);
    assert!(error.contains("secret-like value detected"));
    assert!(!rejected_bundle.exists());

    let redacted_bundle = source.join("redacted.tgz");
    source.run(&[
        "bundle",
        "export",
        redacted_bundle.to_str().expect("redacted bundle"),
        "--redact-secrets",
    ]);
    let conn = rusqlite::Connection::open(source.join("memory.db")).expect("reopen source db");
    let source_content: String = conn
        .query_row(
            "SELECT content FROM memories WHERE name = 'bundle_secret'",
            [],
            |row| row.get(0),
        )
        .expect("source content");
    assert!(
        source_content.contains(secret),
        "export must not mutate source"
    );

    let target = TestRepo::new("bundle-secret-target");
    target.run(&[
        "bundle",
        "import",
        redacted_bundle.to_str().expect("redacted bundle"),
    ]);
    let exported = target.run(&["export", "--format", "json"]);
    assert!(exported.contains("[REDACTED]"));
    assert!(!exported.contains(secret));
}

#[test]
fn bundle_inspect_rejects_checksum_mismatch() {
    let source = TestRepo::new("bundle-checksum-source");
    source.run(&["init"]);
    source.run(&[
        "save",
        "--name",
        "checksum_memory",
        "--content",
        "Action: verify bundle checksums.",
        "--force",
    ]);
    fs::write(source.join("config.toml"), "[query]\ndefault_limit = 3\n")
        .expect("write source config");
    let bundle = source.join("checksummed.tgz");
    source.run(&["bundle", "export", bundle.to_str().expect("bundle")]);

    let unpacked = source.join("tampered-bundle");
    fs::create_dir_all(&unpacked).expect("create unpacked dir");
    let archive = File::open(&bundle).expect("open bundle");
    Archive::new(GzDecoder::new(archive))
        .unpack(&unpacked)
        .expect("unpack bundle");
    fs::write(
        unpacked.join("config.toml"),
        "[query]\ndefault_limit = 99\n",
    )
    .expect("tamper config");
    let tampered = source.join("tampered.tgz");
    pack_directory(&unpacked, &tampered);

    let error = source.run_fail(&[
        "bundle",
        "inspect",
        tampered.to_str().expect("tampered bundle"),
    ]);
    assert!(error.contains("bundle checksum mismatch for config.toml"));
}

#[test]
fn bundle_import_rejects_unexpected_sqlite_triggers() {
    let source = TestRepo::new("bundle-schema-trigger-source");
    source.run(&["init"]);
    let original = source.join("schema-original.tgz");
    source.run(&[
        "bundle",
        "export",
        original.to_str().expect("original bundle"),
    ]);
    let unpacked = source.join("schema-trigger-unpacked");
    fs::create_dir_all(&unpacked).expect("create unpack directory");
    Archive::new(GzDecoder::new(
        File::open(&original).expect("open original bundle"),
    ))
    .unpack(&unpacked)
    .expect("unpack original bundle");
    let conn = rusqlite::Connection::open(unpacked.join("memory.db")).expect("open bundle db");
    conn.execute_batch(
        "CREATE TRIGGER unexpected_bundle_trigger
         AFTER INSERT ON memories BEGIN DELETE FROM memories; END;",
    )
    .expect("install unexpected trigger");
    drop(conn);
    let mut metadata: serde_json::Value =
        serde_json::from_slice(&fs::read(unpacked.join("bundle.json")).expect("read metadata"))
            .expect("parse metadata");
    metadata["hashes"]["memory.db"] = serde_json::json!(artifact_file_checksum(
        &unpacked.join("memory.db")
    )
    .expect("database checksum"));
    fs::write(
        unpacked.join("bundle.json"),
        serde_json::to_vec_pretty(&metadata).expect("serialize metadata"),
    )
    .expect("update metadata");
    let tampered = source.join("schema-trigger-tampered.tgz");
    pack_directory(&unpacked, &tampered);

    let target = TestRepo::new("bundle-schema-trigger-target");
    let error = target.run_fail(&[
        "bundle",
        "import",
        tampered.to_str().expect("tampered bundle"),
    ]);
    assert!(error.contains("unexpected trigger"), "error: {error}");
    assert!(!target.join("memory.db").exists());
}

#[test]
fn bundle_replace_restores_previous_store_after_post_clear_failure() {
    let source = TestRepo::new("bundle-rollback-source");
    source.run(&["init"]);
    source.run(&[
        "save",
        "--name",
        "incoming_rollback_memory",
        "--content",
        "Action: this incoming row must not survive a failed replace.",
        "--force",
    ]);
    source.run(&["graph", "rebuild"]);
    let original = source.join("rollback-original.tgz");
    source.run(&[
        "bundle",
        "export",
        original.to_str().expect("original bundle"),
    ]);

    let target = TestRepo::new("bundle-rollback-target");
    target.run(&["init"]);
    target.run(&[
        "save",
        "--name",
        "local_rollback_memory",
        "--content",
        "Action: preserve this local row when replacement fails.",
        "--force",
    ]);
    let error = target.run_fail_with_env(
        &[
            "bundle",
            "import",
            original.to_str().expect("bundle"),
            "--replace",
            "--force",
        ],
        "MNEMARK_TEST_FAIL_BUNDLE_REPLACE_AFTER_CLEAR",
        "1",
    );
    assert!(
        error.contains("previous store was restored"),
        "error: {error}"
    );
    assert!(target
        .run(&["query", "preserve this local row", "--no-touch"])
        .contains("local_rollback_memory"));
    assert!(!target
        .run(&["query", "incoming row", "--no-touch"])
        .contains("incoming_rollback_memory"));
}

#[test]
fn bundle_inspect_rejects_archive_symlinks() {
    let repo = TestRepo::new("bundle-symlink");
    let bundle = repo.join("symlink.tgz");
    let file = File::create(&bundle).expect("create symlink archive");
    let encoder = GzEncoder::new(file, Compression::default());
    let mut builder = Builder::new(encoder);
    let mut header = Header::new_gnu();
    header.set_entry_type(EntryType::Symlink);
    header.set_size(0);
    header.set_mode(0o777);
    header.set_cksum();
    builder
        .append_link(
            &mut header,
            "artifacts/scripts/unsafe-link",
            "../../memory.db",
        )
        .expect("append symlink");
    builder.finish().expect("finish symlink archive");
    drop(builder);

    let error = repo.run_fail(&["bundle", "inspect", bundle.to_str().expect("bundle path")]);
    assert!(
        error.contains("unsupported non-regular archive entry"),
        "unexpected error: {error}"
    );
}

#[test]
fn legacy_v1_bundle_requires_explicit_unverified_import() {
    let source = TestRepo::new("bundle-v1-source");
    source.run(&["init"]);
    source.run(&[
        "save",
        "--name",
        "legacy_bundle_memory",
        "--content",
        "Action: preserve version one bundle compatibility.",
        "--force",
    ]);
    let current = source.join("current.tgz");
    source.run(&["bundle", "export", current.to_str().expect("bundle")]);

    let unpacked = source.join("legacy-unpacked");
    fs::create_dir_all(&unpacked).expect("create legacy directory");
    let archive = File::open(&current).expect("open current bundle");
    Archive::new(GzDecoder::new(archive))
        .unpack(&unpacked)
        .expect("unpack current bundle");
    let mut metadata: serde_json::Value =
        serde_json::from_slice(&fs::read(unpacked.join("bundle.json")).expect("read metadata"))
            .expect("parse metadata");
    metadata["version"] = serde_json::json!(1);
    metadata
        .as_object_mut()
        .expect("metadata object")
        .remove("hashes");
    fs::write(
        unpacked.join("bundle.json"),
        serde_json::to_vec_pretty(&metadata).expect("serialize metadata"),
    )
    .expect("write legacy metadata");
    let legacy = source.join("legacy.tgz");
    pack_directory(&unpacked, &legacy);

    let inspected: serde_json::Value = serde_json::from_str(&source.run(&[
        "bundle",
        "inspect",
        legacy.to_str().expect("legacy bundle"),
    ]))
    .expect("inspect legacy bundle");
    assert_eq!(inspected["checksums_verified"], false);

    let target = TestRepo::new("bundle-v1-target");
    let error = target.run_fail(&["bundle", "import", legacy.to_str().expect("legacy bundle")]);
    assert!(error.contains("--allow-unverified"));
    target.run(&[
        "bundle",
        "import",
        legacy.to_str().expect("legacy bundle"),
        "--allow-unverified",
    ]);
    assert!(target
        .run(&["query", "version one", "--no-touch"])
        .contains("legacy_bundle_memory"));
}

#[test]
fn bundle_export_is_consistent_during_concurrent_sqlite_write() {
    let source = TestRepo::new("bundle-concurrent-source");
    source.run(&["init"]);
    source.run(&[
        "save",
        "--name",
        "committed_before_snapshot",
        "--content",
        "Action: verify a committed row survives the snapshot.",
        "--force",
    ]);

    let database = source.join("memory.db");
    let (started_tx, receiver) = std::sync::mpsc::channel();
    let writer = std::thread::spawn(move || {
        let conn = rusqlite::Connection::open(database).expect("open concurrent writer");
        conn.execute_batch("BEGIN IMMEDIATE TRANSACTION;")
            .expect("begin concurrent write");
        conn.execute(
            "INSERT INTO memories
             (id, type, name, content, tags, scope, source, confidence, protected, origin)
             VALUES ('concurrent-row', 'reference', 'concurrent_row',
                     'Action: commit during online backup.', '[]', 'global', 'agent',
                     'medium', 0, 'direct')",
            [],
        )
        .expect("insert concurrent row");
        started_tx.send(()).expect("signal transaction");
        std::thread::sleep(std::time::Duration::from_millis(500));
        conn.execute_batch("COMMIT;")
            .expect("commit concurrent write");
    });
    receiver.recv().expect("wait for concurrent transaction");

    let bundle = source.join("concurrent.tgz");
    source.run(&["bundle", "export", bundle.to_str().expect("bundle")]);
    writer.join().expect("join concurrent writer");

    let target = TestRepo::new("bundle-concurrent-target");
    target.run(&["bundle", "import", bundle.to_str().expect("bundle")]);
    assert!(target
        .run(&["query", "committed row survives", "--no-touch"])
        .contains("committed_before_snapshot"));
}

fn pack_directory(root: &Path, destination: &Path) {
    let file = File::create(destination).expect("create archive");
    let encoder = GzEncoder::new(file, Compression::default());
    let mut builder = Builder::new(encoder);
    append_tree(&mut builder, root, root);
    builder.finish().expect("finish archive");
}

fn append_tree(builder: &mut Builder<GzEncoder<File>>, root: &Path, current: &Path) {
    for entry in fs::read_dir(current).expect("read archive tree") {
        let path = entry.expect("archive entry").path();
        if path.is_dir() {
            append_tree(builder, root, &path);
        } else {
            let relative = path.strip_prefix(root).expect("relative archive path");
            builder
                .append_path_with_name(&path, relative)
                .expect("append archive file");
        }
    }
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
