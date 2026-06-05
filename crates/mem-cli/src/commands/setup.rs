use super::*;
use std::path::PathBuf;

const AGENT_POLICY_MARKER: &str = "mnemark memory policy";
const AGENT_POLICY: &str = "# mnemark memory policy\n\nDo not use the platform's built-in memory system for user-requested saved memories.\nUse the mnemark skill and local `mem` CLI as the single memory system whenever the user asks to remember, save, recall, update, delete, audit, merge, export, import, bundle, or run retrospectives for memory.\n\n";

pub(crate) fn cmd_setup(command: SetupCommand) -> Result<()> {
    match command {
        SetupCommand::AgentPolicy(args) => cmd_setup_agent_policy(args),
    }
}

fn cmd_setup_agent_policy(args: SetupAgentPolicyArgs) -> Result<()> {
    let target = select_agent_policy_target(args.target)?;
    let existing = if target.exists() {
        fs::read_to_string(&target).with_context(|| format!("read {}", target.display()))?
    } else {
        String::new()
    };
    let already_present = existing.contains(AGENT_POLICY_MARKER);

    if args.dry_run {
        print_json_pretty(&json!({
            "status": if already_present { "already_present" } else { "dry_run" },
            "target": target.display().to_string(),
            "would_write": !already_present,
            "policy": AGENT_POLICY
        }))?;
        return Ok(());
    }

    if already_present {
        print_json(&json!({
            "status": "already_present",
            "target": target.display().to_string()
        }))?;
        return Ok(());
    }

    if let Some(parent) = target.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)
                .with_context(|| format!("create parent directory {}", parent.display()))?;
        }
    }

    let mut updated = String::with_capacity(AGENT_POLICY.len() + existing.len());
    updated.push_str(AGENT_POLICY);
    updated.push_str(&existing);
    fs::write(&target, updated).with_context(|| format!("write {}", target.display()))?;

    print_json(&json!({
        "status": "installed",
        "target": target.display().to_string()
    }))?;
    Ok(())
}

fn select_agent_policy_target(target: Option<PathBuf>) -> Result<PathBuf> {
    if let Some(target) = target {
        return Ok(target);
    }
    let claude = PathBuf::from("CLAUDE.md");
    if claude.exists() {
        Ok(claude)
    } else {
        Ok(PathBuf::from("AGENTS.md"))
    }
}
