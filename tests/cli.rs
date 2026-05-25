use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn mem_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_mem"))
}

fn temp_repo(name: &str) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("agent-knowledge-{name}-{stamp}"));
    fs::create_dir_all(dir.join("schema")).expect("schema dir");
    fs::write(
        dir.join("schema/memory-schema.sql"),
        include_str!("../schema/memory-schema.sql"),
    )
    .expect("schema");
    dir
}

fn run(repo: &PathBuf, args: &[&str]) -> String {
    let output = Command::new(mem_bin())
        .current_dir(repo)
        .args(args)
        .output()
        .expect("run mem");
    assert!(
        output.status.success(),
        "command failed: {:?}\nstdout={}\nstderr={}",
        args,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("utf8 stdout")
}

#[test]
fn save_query_and_version_conflict() {
    let repo = temp_repo("save-query");
    run(&repo, &["init"]);
    let saved = run(
        &repo,
        &[
            "save",
            "--type",
            "feedback",
            "--name",
            "no_emoji",
            "--scope",
            "global",
            "--source",
            "manual",
            "--tags",
            r#"["style:no-emoji"]"#,
            "--content",
            "不要使用 emoji",
        ],
    );
    assert!(saved.contains(r#""status":"saved""#));

    let query = run(&repo, &["query", "使用"]);
    assert!(query.contains("no_emoji"));

    let conflict = run(
        &repo,
        &[
            "update",
            "no_emoji",
            "--expected-version",
            "99",
            "--source",
            "manual",
            "--content",
            "不要使用 emoji",
        ],
    );
    assert!(conflict.contains("version_conflict"));

    fs::remove_dir_all(repo).ok();
}

#[test]
fn retro_bundle_contains_repository_state() {
    let repo = temp_repo("retro");
    run(&repo, &["init"]);
    let retro = run(&repo, &["retro", "daily"]);
    assert!(retro.contains("retro_bundle"));
    assert!(retro.contains("platform-provided"));
    fs::remove_dir_all(repo).ok();
}
