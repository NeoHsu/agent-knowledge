use std::fs;

#[cfg(unix)]
use std::os::unix::fs::{PermissionsExt, symlink};

#[cfg(unix)]
use crate::support::temp_path;
use crate::support::{TestRepo, synthetic_generic_secret, synthetic_github_token};

const HELLO_SHA256: &str =
    "sha256:5891b5b522d5df086d0ff0b110fbd9d21bb4fc7163af34d08286a2e846f6be03";
const ABC_SHA256: &str = "sha256:ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

#[test]
fn artifact_list_missing_manifest_is_empty() {
    let repo = TestRepo::new("artifact-missing-manifest");

    let output = repo.run(&["artifact", "list"]);
    let rows: serde_json::Value = serde_json::from_str(&output).expect("artifact list json");

    assert_eq!(rows.as_array().expect("rows").len(), 0);
}

#[test]
fn artifact_check_list_and_show_manifest_entries() {
    let repo = TestRepo::new("artifact-check-success");
    write_file(repo.join("artifacts/scripts/ci-triage.sh"), "hello\n", true);
    fs::write(
        repo.join("manifest.toml"),
        format!(
            r#"
version = 1

[artifacts.scripts.ci-triage]
path = "artifacts/scripts/ci-triage.sh"
kind = "script"
scope = "global"
checksum = "{HELLO_SHA256}"
description = "Collect CI failure context."
executable = true
"#
        ),
    )
    .expect("write manifest");

    let checked: serde_json::Value =
        serde_json::from_str(&repo.run(&["artifact", "check"])).expect("check json");
    assert_eq!(checked["status"], "ok");
    assert_eq!(checked["checked"], 1);

    let listed: serde_json::Value =
        serde_json::from_str(&repo.run(&["artifact", "list"])).expect("list json");
    assert_eq!(listed[0]["name"], "scripts.ci-triage");
    assert_eq!(listed[0]["short_name"], "ci-triage");

    let shown: serde_json::Value =
        serde_json::from_str(&repo.run(&["artifact", "show", "ci-triage"])).expect("show json");
    assert_eq!(shown["name"], "scripts.ci-triage");
    assert_eq!(shown["kind"], "script");
}

#[test]
fn artifact_check_reports_problems_without_executing_scripts() {
    let repo = TestRepo::new("artifact-check-problems");
    let marker = repo.join("executed-marker");
    let script = format!("#!/bin/sh\ntouch {}\n", marker.display());
    write_file(repo.join("artifacts/scripts/mismatch.sh"), &script, false);
    fs::write(
        repo.join("manifest.toml"),
        format!(
            r#"
version = 1

[artifacts.scripts.missing]
path = "artifacts/scripts/missing.sh"
kind = "script"
scope = "global"
checksum = "{ABC_SHA256}"
executable = true

[artifacts.scripts.mismatch]
path = "artifacts/scripts/mismatch.sh"
kind = "script"
scope = "global"
checksum = "{ABC_SHA256}"
executable = true

[artifacts.scripts.escape]
path = "../escape.sh"
kind = "script"
scope = "global"
checksum = "{ABC_SHA256}"
executable = true
"#
        ),
    )
    .expect("write manifest");

    let checked: serde_json::Value =
        serde_json::from_str(&repo.run(&["artifact", "check"])).expect("check json");

    assert_eq!(checked["status"], "error");
    assert!(array_contains(&checked["missing"], "scripts.missing"));
    assert_eq!(checked["checksum_mismatch"][0]["name"], "scripts.mismatch");
    #[cfg(unix)]
    assert!(array_contains(
        &checked["not_executable"],
        "scripts.mismatch"
    ));
    assert_eq!(checked["unsafe_paths"][0]["name"], "scripts.escape");
    assert!(!marker.exists(), "artifact check must not execute scripts");
}

#[test]
fn artifact_add_update_and_remove_manage_manifest() {
    let repo = TestRepo::new("artifact-registration");
    let artifact_path = repo.join("artifacts/scripts/ci-triage.sh");
    write_file(&artifact_path, "hello\n", true);

    let added: serde_json::Value = serde_json::from_str(&repo.run(&[
        "artifact",
        "add",
        "artifacts/scripts/ci-triage.sh",
        "--name",
        "ci-helper",
        "--kind",
        "script",
        "--scope",
        "global",
        "--description",
        "Collect CI failure context.",
        "--tags",
        r#"["tool:gh","workflow:ci-triage"]"#,
        "--executable",
    ]))
    .expect("add json");
    assert_eq!(added["name"], "scripts.ci-helper");
    assert_eq!(added["checksum"], HELLO_SHA256);
    assert!(repo.join("manifest.toml").exists());

    fs::write(&artifact_path, "changed\n").expect("change artifact");
    #[cfg(unix)]
    {
        let mut permissions = fs::metadata(&artifact_path)
            .expect("metadata")
            .permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&artifact_path, permissions).expect("chmod artifact");
    }

    let updated: serde_json::Value =
        serde_json::from_str(&repo.run(&["artifact", "update", "ci-helper", "--checksum"]))
            .expect("update json");
    assert_eq!(updated["name"], "scripts.ci-helper");
    assert_ne!(updated["checksum"], HELLO_SHA256);

    let checked: serde_json::Value =
        serde_json::from_str(&repo.run(&["artifact", "check"])).expect("check json");
    assert_eq!(checked["status"], "ok");

    let removed: serde_json::Value =
        serde_json::from_str(&repo.run(&["artifact", "remove", "ci-helper"])).expect("remove json");
    assert_eq!(removed["name"], "scripts.ci-helper");
    assert!(artifact_path.exists(), "remove should keep file by default");
    let listed: serde_json::Value =
        serde_json::from_str(&repo.run(&["artifact", "list"])).expect("list json");
    assert_eq!(listed.as_array().expect("list").len(), 0);

    repo.run(&[
        "artifact",
        "add",
        "artifacts/scripts/ci-triage.sh",
        "--kind",
        "script",
        "--scope",
        "global",
        "--executable",
    ]);
    repo.run(&["artifact", "remove", "scripts.ci-triage", "--delete-file"]);
    assert!(
        !artifact_path.exists(),
        "remove --delete-file should delete file"
    );
}

fn write_file(path: impl AsRef<std::path::Path>, content: &str, executable: bool) {
    let path = path.as_ref();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("artifact dir");
    }
    fs::write(path, content).expect("write artifact");
    #[cfg(unix)]
    if executable {
        let mut permissions = fs::metadata(path).expect("metadata").permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(path, permissions).expect("chmod artifact");
    }
    #[cfg(not(unix))]
    let _ = executable;
}

#[cfg(unix)]
#[test]
fn artifact_redaction_rejects_symlinks_before_touching_external_files() {
    let repo = TestRepo::new("artifact-symlink-redaction");
    repo.run(&["init"]);
    let outside = temp_path("artifact-symlink-outside");
    fs::create_dir_all(&outside).expect("outside directory");
    let secret = synthetic_generic_secret();

    let external_file = outside.join("external.txt");
    fs::write(&external_file, &secret).expect("external file");
    fs::create_dir_all(repo.join("artifacts/snippets")).expect("artifact directory");
    symlink(&external_file, repo.join("artifacts/snippets/direct.txt"))
        .expect("direct artifact symlink");

    let direct_error = repo.run_fail(&[
        "artifact",
        "add",
        "artifacts/snippets/direct.txt",
        "--kind",
        "snippet",
        "--redact-secrets",
    ]);
    assert!(direct_error.contains("refusing artifact symlink"));
    assert_eq!(
        fs::read_to_string(&external_file).expect("external content"),
        secret
    );

    fs::remove_dir_all(repo.join("artifacts/snippets")).expect("remove artifact directory");
    symlink(&outside, repo.join("artifacts/snippets")).expect("intermediate artifact symlink");
    let intermediate_error = repo.run_fail(&[
        "artifact",
        "add",
        "artifacts/snippets/external.txt",
        "--kind",
        "snippet",
        "--redact-secrets",
    ]);
    assert!(intermediate_error.contains("refusing artifact symlink"));
    assert_eq!(
        fs::read_to_string(&external_file).expect("external content"),
        secret
    );
    assert!(!repo.join("manifest.toml").exists());

    fs::remove_dir_all(outside).ok();
}

#[test]
fn artifact_secret_policy_requires_explicit_redaction() {
    let repo = TestRepo::new("artifact-secret-policy");
    let artifact_path = repo.join("artifacts/snippets/credentials.txt");
    let secret = synthetic_github_token();
    write_file(&artifact_path, &format!("token={secret}\n"), false);

    let rejected = repo.run_fail(&[
        "artifact",
        "add",
        "artifacts/snippets/credentials.txt",
        "--kind",
        "snippet",
        "--scope",
        "global",
    ]);
    assert!(rejected.contains("secret-like value detected in artifact file"));
    assert!(!repo.join("manifest.toml").exists());

    let added: serde_json::Value = serde_json::from_str(&repo.run(&[
        "artifact",
        "add",
        "artifacts/snippets/credentials.txt",
        "--kind",
        "snippet",
        "--scope",
        "global",
        "--description",
        &format!("redact {secret}"),
        "--redact-secrets",
    ]))
    .expect("redacted artifact json");
    assert_eq!(added["name"], "snippets.credentials");
    let content = fs::read_to_string(&artifact_path).expect("redacted artifact");
    assert!(content.contains("[REDACTED]"));
    assert!(!content.contains(&secret));
    let manifest = fs::read_to_string(repo.join("manifest.toml")).expect("manifest");
    assert!(manifest.contains("[REDACTED]"));
    assert!(!manifest.contains(&secret));
}

fn array_contains(value: &serde_json::Value, wanted: &str) -> bool {
    value
        .as_array()
        .expect("array")
        .iter()
        .any(|value| value.as_str() == Some(wanted))
}
