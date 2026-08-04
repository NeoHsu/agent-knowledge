mod hook;
mod platform;
mod policy;
mod skill;

use super::*;
use hook::wire_claude_hook;
use policy::install_policy;
use skill::install_shared_skill;

pub(crate) use hook::{HOOK_COMMAND, LEGACY_HOOK_COMMAND};
pub(crate) use platform::{PLATFORMS, PlatformSpec, SHARED_SKILLS_DIR, base_dir, platform_by_name};
pub(crate) use policy::{
    POLICY_MARKER_V2, POLICY_MARKER_V3, POLICY_MARKER_V4, POLICY_MARKER_V5, has_current_policy,
    has_v4_policy,
};
pub(crate) use skill::{skill_files_current, skill_link_points_to};

pub(crate) fn cmd_setup(command: SetupCommand) -> Result<()> {
    match command {
        SetupCommand::List => cmd_setup_list(),
        SetupCommand::ClaudeCode(args) => cmd_setup_platform("claude-code", args),
        SetupCommand::Codex(args) => cmd_setup_platform("codex", args),
        SetupCommand::Pi(args) => cmd_setup_platform("pi", args),
        SetupCommand::GeminiCli(args) => cmd_setup_platform("gemini-cli", args),
        SetupCommand::Opencode(args) => cmd_setup_platform("opencode", args),
    }
}

fn cmd_setup_list() -> Result<()> {
    let base = base_dir(None);
    let shared_skill = base.join(SHARED_SKILLS_DIR).join("mnemark");
    let rows = PLATFORMS
        .iter()
        .map(|platform| {
            json!({
                "name": platform.name,
                "instructions": base.join(platform.instructions).display().to_string(),
                "shared_skill": shared_skill.display().to_string(),
                "skills_dir": platform
                    .skills_dir
                    .map(|dir| base.join(dir).join("mnemark").display().to_string()),
                "skill_mode": match platform.skills_dir {
                    Some(SHARED_SKILLS_DIR) => "shared",
                    Some(_) => "symlink",
                    None => "unsupported"
                },
                "session_hook": platform
                    .claude_settings
                    .map(|path| base.join(path).display().to_string()),
                "session_start": if platform.claude_settings.is_some() {
                    "hook"
                } else {
                    "policy_prose (mem prime)"
                }
            })
        })
        .collect::<Vec<_>>();
    print_json_pretty(&rows)
}

fn cmd_setup_platform(name: &str, args: SetupPlatformArgs) -> Result<()> {
    let platform = platform_by_name(name).ok_or_else(|| anyhow!("unknown platform: {name}"))?;
    let base = base_dir(args.base_dir.as_deref());
    let instructions = args
        .instructions
        .unwrap_or_else(|| base.join(platform.instructions));
    let platform_skill_root = args
        .skills_dir
        .or_else(|| platform.skills_dir.map(|dir| base.join(dir)))
        .map(|parent| parent.join("mnemark"));
    let shared_skill_root = args
        .shared_skills_dir
        .unwrap_or_else(|| base.join(SHARED_SKILLS_DIR))
        .join("mnemark");

    let policy = install_policy(&instructions, args.dry_run)?;
    let skill = if args.no_skill {
        json!({"status": "skipped"})
    } else {
        install_shared_skill(
            &shared_skill_root,
            platform_skill_root.as_deref(),
            args.dry_run,
        )?
    };
    let hook = if args.no_hook {
        json!({"status": "skipped"})
    } else if let Some(settings) = platform.claude_settings {
        wire_claude_hook(&base.join(settings), args.dry_run)?
    } else {
        json!({
            "status": "policy_prose",
            "detail": "no session-start hook mechanism; the policy block instructs agents to run `mem prime`"
        })
    };

    print_json_pretty(&json!({
        "status": if args.dry_run { "dry_run" } else { "configured" },
        "platform": platform.name,
        "policy": policy,
        "skill": skill,
        "session_hook": hook
    }))
}
