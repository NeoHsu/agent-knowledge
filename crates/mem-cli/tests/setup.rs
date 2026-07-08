use std::fs;
use std::process::Command;

mod support;

use support::{mem_bin, temp_path, TestRepo};

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

#[test]
fn setup_claude_code_wires_policy_skill_and_hook() {
    let repo = TestRepo::new("setup-claude-code");
    let base = temp_path("setup-claude-base");
    fs::create_dir_all(&base).expect("base dir");
    let base_str = base.to_str().expect("base path");

    let output = repo.run(&["setup", "claude-code", "--base-dir", base_str]);
    let result: serde_json::Value = serde_json::from_str(&output).expect("setup json");
    assert_eq!(result["status"], "configured");
    assert_eq!(result["policy"]["status"], "installed");
    assert_eq!(result["skill"]["status"], "installed");
    assert_eq!(result["session_hook"]["status"], "installed");

    let policy = fs::read_to_string(base.join(".claude/CLAUDE.md")).expect("policy file");
    assert!(policy.contains("<!-- mnemark-policy v2 -->"));
    assert!(policy.contains("mem prime"));
    assert!(base.join(".claude/skills/mnemark/SKILL.md").exists());
    assert!(base
        .join(".claude/skills/mnemark/references/cli-guide.md")
        .exists());
    assert!(base
        .join(".claude/skills/mnemark/references/memory-quality.md")
        .exists());

    let settings: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(base.join(".claude/settings.json")).expect("settings"),
    )
    .expect("settings json");
    let command = settings["hooks"]["SessionStart"][0]["hooks"][0]["command"]
        .as_str()
        .expect("hook command");
    assert!(command.contains("mem prime"));

    // Second run is idempotent.
    let output = repo.run(&["setup", "claude-code", "--base-dir", base_str]);
    let result: serde_json::Value = serde_json::from_str(&output).expect("setup json");
    assert_eq!(result["policy"]["status"], "already_present");
    assert_eq!(result["skill"]["status"], "up_to_date");
    assert_eq!(result["session_hook"]["status"], "already_present");

    fs::remove_dir_all(base).ok();
}

#[test]
fn setup_claude_code_preserves_existing_settings() {
    let repo = TestRepo::new("setup-claude-preserve");
    let base = temp_path("setup-claude-preserve-base");
    fs::create_dir_all(base.join(".claude")).expect("base dir");
    fs::write(
        base.join(".claude/settings.json"),
        "{\n  \"model\": \"opus\",\n  \"hooks\": {\n    \"Stop\": []\n  }\n}\n",
    )
    .expect("seed settings");

    repo.run(&[
        "setup",
        "claude-code",
        "--base-dir",
        base.to_str().expect("base"),
        "--no-skill",
    ]);

    let settings: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(base.join(".claude/settings.json")).expect("settings"),
    )
    .expect("settings json");
    assert_eq!(settings["model"], "opus");
    assert!(settings["hooks"]["Stop"].is_array());
    assert!(settings["hooks"]["SessionStart"][0]["hooks"][0]["command"]
        .as_str()
        .expect("command")
        .contains("mem prime"));

    fs::remove_dir_all(base).ok();
}

#[test]
fn setup_upgrades_legacy_policy_block() {
    let repo = TestRepo::new("setup-upgrade-legacy");
    let base = temp_path("setup-upgrade-base");
    fs::create_dir_all(base.join(".codex")).expect("base dir");
    let legacy = "# mnemark memory policy\n\nDo not use the platform's built-in memory system for user-requested saved memories.\nUse the mnemark skill and local `mem` CLI as the single memory system whenever the user asks to remember, save, recall, update, delete, audit, merge, export, import, bundle, or run retrospectives for memory.\n\n";
    fs::write(
        base.join(".codex/AGENTS.md"),
        format!("{legacy}# Keep\n\nuser content\n"),
    )
    .expect("seed agents");

    let output = repo.run(&[
        "setup",
        "codex",
        "--base-dir",
        base.to_str().expect("base"),
        "--no-skill",
    ]);
    let result: serde_json::Value = serde_json::from_str(&output).expect("setup json");
    assert_eq!(result["policy"]["status"], "upgraded");
    assert_eq!(result["session_hook"]["status"], "policy_prose");

    let content = fs::read_to_string(base.join(".codex/AGENTS.md")).expect("agents");
    assert!(content.contains("<!-- mnemark-policy v2 -->"));
    assert!(content.contains("# Keep\n\nuser content"));
    assert!(!content.contains("whenever the user asks to remember, save, recall"));

    fs::remove_dir_all(base).ok();
}

#[test]
fn setup_gemini_has_no_skill_dir_and_list_reports_platforms() {
    let repo = TestRepo::new("setup-gemini");
    let base = temp_path("setup-gemini-base");
    fs::create_dir_all(&base).expect("base dir");

    let output = repo.run(&[
        "setup",
        "gemini-cli",
        "--base-dir",
        base.to_str().expect("base"),
    ]);
    let result: serde_json::Value = serde_json::from_str(&output).expect("setup json");
    assert_eq!(result["skill"]["status"], "unsupported");
    assert!(base.join(".gemini/GEMINI.md").exists());

    let listing = repo.run(&["setup", "list"]);
    let platforms: serde_json::Value = serde_json::from_str(&listing).expect("list json");
    let names: Vec<&str> = platforms
        .as_array()
        .expect("array")
        .iter()
        .map(|p| p["name"].as_str().expect("name"))
        .collect();
    assert_eq!(
        names,
        vec!["claude-code", "codex", "gemini-cli", "opencode"]
    );

    fs::remove_dir_all(base).ok();
}
