use std::fs;
use std::process::Command;

mod support;

use support::{mem_bin, TestRepo};

#[test]
fn setup_agent_policy_creates_agents_md_by_default() {
    let repo = TestRepo::new("setup-agent-policy-default");

    let output = repo.run(&["setup", "agent-policy"]);
    let result: serde_json::Value = serde_json::from_str(&output).expect("setup json");
    assert_eq!(result["status"], "installed");
    assert_eq!(result["target"], "AGENTS.md");

    let content = fs::read_to_string(repo.join("AGENTS.md")).expect("agents file");
    assert!(content.starts_with("# mnemark memory policy"));
    assert!(content.contains("Use the mnemark skill and local `mem` CLI"));

    let second = repo.run(&["setup", "agent-policy"]);
    let second: serde_json::Value = serde_json::from_str(&second).expect("setup json");
    assert_eq!(second["status"], "already_present");
    let content_after = fs::read_to_string(repo.join("AGENTS.md")).expect("agents file");
    assert_eq!(content, content_after);
}

#[test]
fn setup_agent_policy_prefers_existing_claude_and_prepends() {
    let repo = TestRepo::new("setup-agent-policy-claude");
    fs::write(repo.join("CLAUDE.md"), "# Existing\n\nKeep this.\n").expect("write claude");

    let output = repo.run(&["setup", "agent-policy"]);
    let result: serde_json::Value = serde_json::from_str(&output).expect("setup json");
    assert_eq!(result["status"], "installed");
    assert_eq!(result["target"], "CLAUDE.md");

    let content = fs::read_to_string(repo.join("CLAUDE.md")).expect("claude file");
    assert!(content.starts_with("# mnemark memory policy"));
    assert!(content.contains("# Existing\n\nKeep this."));
}

#[test]
fn setup_agent_policy_supports_target_and_dry_run() {
    let repo = TestRepo::new("setup-agent-policy-target");
    let target = repo.join("nested/AGENTS.md");

    let output = Command::new(mem_bin())
        .current_dir(repo.path())
        .args([
            "setup",
            "agent-policy",
            "--target",
            target.to_str().expect("target path"),
            "--dry-run",
        ])
        .output()
        .expect("run dry-run");
    assert!(
        output.status.success(),
        "dry-run failed stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let result: serde_json::Value = serde_json::from_slice(&output.stdout).expect("dry-run json");
    assert_eq!(result["status"], "dry_run");
    assert_eq!(result["would_write"], true);
    assert!(!target.exists());

    let output = Command::new(mem_bin())
        .current_dir(repo.path())
        .args([
            "setup",
            "agent-policy",
            "--target",
            target.to_str().expect("target path"),
        ])
        .output()
        .expect("run setup");
    assert!(
        output.status.success(),
        "setup failed stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(target.exists());
}
