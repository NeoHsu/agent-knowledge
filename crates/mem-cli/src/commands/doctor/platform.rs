use super::super::*;
use super::report::check;
use crate::commands::setup::{
    HOOK_COMMAND, LEGACY_HOOK_COMMAND, POLICY_MARKER_V2, POLICY_MARKER_V3, POLICY_MARKER_V4,
    POLICY_MARKER_V5, PlatformSpec, has_current_policy, has_v4_policy, skill_files_current,
    skill_link_points_to,
};

pub(super) fn check_shared_skill(checks: &mut Vec<Value>, shared_root: &Path) {
    if skill_files_current(shared_root) {
        checks.push(check(
            "shared.skill",
            "ok",
            format!("shared mnemark skill at {}", shared_root.display()),
            None,
        ));
    } else if shared_root.exists() {
        checks.push(check(
            "shared.skill",
            "warn",
            format!(
                "shared mnemark skill is incomplete or stale at {}",
                shared_root.display()
            ),
            Some("run `mem setup <platform>` to refresh the shared skill"),
        ));
    } else {
        checks.push(check(
            "shared.skill",
            "missing",
            format!(
                "shared mnemark skill not found at {}",
                shared_root.display()
            ),
            Some("run `mem setup <platform>` to install the shared skill"),
        ));
    }
}

pub(super) fn check_platform(
    checks: &mut Vec<Value>,
    platform: &PlatformSpec,
    base: &Path,
    shared_skill_root: &Path,
) {
    let prefix = platform.name;
    let instructions = base.join(platform.instructions);
    match fs::read_to_string(&instructions) {
        Ok(content) if has_current_policy(&content) => checks.push(check(
            format!("{prefix}.policy"),
            "ok",
            format!("v5 policy in {}", instructions.display()),
            None,
        )),
        Ok(content) if content.contains(POLICY_MARKER_V5) => checks.push(check(
            format!("{prefix}.policy"),
            "warn",
            format!("drifted v5 policy in {}", instructions.display()),
            Some("replace the managed block with the policy from `mem setup <platform> --dry-run`"),
        )),
        Ok(content) if has_v4_policy(&content) => checks.push(check(
            format!("{prefix}.policy"),
            "warn",
            format!("v4 policy in {}", instructions.display()),
            Some("run `mem setup <platform>` to upgrade the policy block to v5"),
        )),
        Ok(content) if content.contains(POLICY_MARKER_V4) => checks.push(check(
            format!("{prefix}.policy"),
            "warn",
            format!("drifted v4 policy in {}", instructions.display()),
            Some("replace the managed block with the policy from `mem setup <platform> --dry-run`"),
        )),
        Ok(content) if content.contains(POLICY_MARKER_V3) => checks.push(check(
            format!("{prefix}.policy"),
            "warn",
            format!("v3 policy in {}", instructions.display()),
            Some("run `mem setup <platform>` to upgrade the policy block to v5"),
        )),
        Ok(content) if content.contains(POLICY_MARKER_V2) => checks.push(check(
            format!("{prefix}.policy"),
            "warn",
            format!("v2 policy in {}", instructions.display()),
            Some("run `mem setup <platform>` to upgrade the policy block to v5"),
        )),
        Ok(content) if content.contains("mnemark memory policy") => checks.push(check(
            format!("{prefix}.policy"),
            "warn",
            format!("legacy policy block in {}", instructions.display()),
            Some("run `mem setup <platform>` and replace the old block with v5"),
        )),
        Ok(_) => checks.push(check(
            format!("{prefix}.policy"),
            "warn",
            format!("{} exists without a mnemark policy", instructions.display()),
            Some("run `mem setup <platform>`"),
        )),
        Err(_) => checks.push(check(
            format!("{prefix}.policy"),
            "missing",
            format!("{} not found", instructions.display()),
            Some("run `mem setup <platform>` (skip if this agent is not installed)"),
        )),
    }

    if let Some(skills_dir) = platform.skills_dir {
        let platform_skill_root = base.join(skills_dir).join("mnemark");
        if platform_skill_root == shared_skill_root {
            checks.push(check(
                format!("{prefix}.skill"),
                if skill_files_current(shared_skill_root) {
                    "ok"
                } else {
                    "warn"
                },
                format!(
                    "platform uses shared skill at {}",
                    shared_skill_root.display()
                ),
                if skill_files_current(shared_skill_root) {
                    None
                } else {
                    Some("run `mem setup <platform>` to refresh the shared skill")
                },
            ));
        } else {
            match fs::symlink_metadata(&platform_skill_root) {
                Ok(metadata) if metadata.file_type().is_symlink() => {
                    if skill_link_points_to(&platform_skill_root, shared_skill_root) {
                        checks.push(check(
                            format!("{prefix}.skill"),
                            "ok",
                            format!(
                                "skill link {} -> {}",
                                platform_skill_root.display(),
                                shared_skill_root.display()
                            ),
                            None,
                        ));
                    } else {
                        checks.push(check(
                            format!("{prefix}.skill"),
                            "warn",
                            format!(
                                "skill symlink {} points somewhere other than {}",
                                platform_skill_root.display(),
                                shared_skill_root.display()
                            ),
                            Some("remove the conflicting link, then run `mem setup <platform>`"),
                        ));
                    }
                }
                Ok(metadata) if metadata.is_dir() => checks.push(check(
                    format!("{prefix}.skill"),
                    "warn",
                    format!("legacy skill copy at {}", platform_skill_root.display()),
                    Some("run `mem setup <platform>` to migrate the managed copy to a shared link"),
                )),
                Ok(_) => checks.push(check(
                    format!("{prefix}.skill"),
                    "warn",
                    format!("invalid skill path at {}", platform_skill_root.display()),
                    Some("move the conflicting path, then run `mem setup <platform>`"),
                )),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => checks.push(check(
                    format!("{prefix}.skill"),
                    "missing",
                    format!("skill link not found at {}", platform_skill_root.display()),
                    Some("run `mem setup <platform>`"),
                )),
                Err(error) => checks.push(check(
                    format!("{prefix}.skill"),
                    "error",
                    format!("cannot inspect {}: {error}", platform_skill_root.display()),
                    None,
                )),
            }
        }
    }

    if let Some(settings_rel) = platform.claude_settings {
        let settings = base.join(settings_rel);
        let status = fs::read_to_string(&settings)
            .ok()
            .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
            .map(|root| {
                let mut current = false;
                let mut legacy = false;
                let mut custom = false;
                if let Some(entries) = root
                    .pointer("/hooks/SessionStart")
                    .and_then(Value::as_array)
                {
                    for entry in entries {
                        let Some(hooks) = entry.get("hooks").and_then(Value::as_array) else {
                            continue;
                        };
                        for hook in hooks {
                            let Some(command) = hook.get("command").and_then(Value::as_str) else {
                                continue;
                            };
                            if command == HOOK_COMMAND {
                                current = true;
                            } else if command == LEGACY_HOOK_COMMAND {
                                legacy = true;
                            } else if command.contains("mem prime")
                                || command.contains("session-prime")
                            {
                                custom = true;
                            }
                        }
                    }
                }
                if current {
                    "current"
                } else if legacy {
                    "legacy"
                } else if custom {
                    "custom"
                } else {
                    "absent"
                }
            });
        match status {
            Some("current") => checks.push(check(
                format!("{prefix}.session_hook"),
                "ok",
                format!("SessionStart hook in {}", settings.display()),
                None,
            )),
            Some("legacy") => checks.push(check(
                format!("{prefix}.session_hook"),
                "warn",
                format!(
                    "legacy SessionStart hook hides `mem prime` failures in {}",
                    settings.display()
                ),
                Some("run `mem setup claude-code` to upgrade the hook"),
            )),
            Some("custom") => checks.push(check(
                format!("{prefix}.session_hook"),
                "ok",
                format!("custom mnemark SessionStart hook in {}", settings.display()),
                None,
            )),
            Some(_) => checks.push(check(
                format!("{prefix}.session_hook"),
                "warn",
                format!("no mnemark SessionStart hook in {}", settings.display()),
                Some("run `mem setup claude-code` to add the `mem prime` hook"),
            )),
            None => checks.push(check(
                format!("{prefix}.session_hook"),
                "missing",
                format!("{} missing or not valid JSON", settings.display()),
                Some(&format!(
                    "run `mem setup claude-code`; the hook command is `{HOOK_COMMAND}`"
                )),
            )),
        }
    }
}
