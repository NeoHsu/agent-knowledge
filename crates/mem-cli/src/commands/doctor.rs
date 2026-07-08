use super::*;
use crate::commands::setup::{
    base_dir, platform_by_name, PlatformSpec, HOOK_COMMAND, PLATFORMS, POLICY_MARKER_V2,
    POLICY_MARKER_V3,
};

pub(crate) fn cmd_doctor(app: &App, args: DoctorArgs) -> Result<()> {
    let mut checks = Vec::new();

    checks.push(check(
        "binary",
        "ok",
        format!(
            "mem {} at {}",
            env!("CARGO_PKG_VERSION"),
            std::env::current_exe()
                .map(|path| path.display().to_string())
                .unwrap_or_else(|_| "unknown".to_string())
        ),
        None,
    ));

    if app.db_path.exists() {
        let conn = app.conn()?;
        let active = memory_count(&conn)?;
        checks.push(check(
            "store",
            "ok",
            format!(
                "root {} ({}), {} active memories",
                app.root.display(),
                app.store_source.as_str(),
                active
            ),
            None,
        ));
        if memory_index::is_stale(app) {
            checks.push(check(
                "index",
                "warn",
                "search index is stale".to_string(),
                Some("run `mem reindex`"),
            ));
        } else {
            checks.push(check(
                "index",
                "ok",
                "search index present".to_string(),
                None,
            ));
        }
        let git_dir = app.root.join(".git");
        if git_dir.exists() {
            checks.push(check(
                "store_git",
                "ok",
                "store is version-controlled".to_string(),
                None,
            ));
        } else {
            checks.push(check(
                "store_git",
                "warn",
                "store is not a git repository; history and `mem sync` are unavailable".to_string(),
                Some("git init the store root, ignore index/ and lock/WAL files, commit"),
            ));
        }
    } else {
        checks.push(check(
            "store",
            "error",
            format!("no memory store at {}", app.root.display()),
            Some("run `mem init`"),
        ));
    }

    let base = base_dir(args.base_dir.as_deref());
    let platforms: Vec<&PlatformSpec> = match args.platform.as_deref() {
        Some(name) => {
            vec![platform_by_name(name).ok_or_else(|| anyhow!("unknown platform: {name}"))?]
        }
        None => PLATFORMS.iter().collect(),
    };
    for platform in platforms {
        check_platform(&mut checks, platform, &base);
    }

    let has_error = checks
        .iter()
        .any(|entry| entry.get("status").and_then(Value::as_str) == Some("error"));
    let has_warn = checks
        .iter()
        .any(|entry| entry.get("status").and_then(Value::as_str) == Some("warn"));
    print_json_pretty(&json!({
        "status": if has_error { "error" } else if has_warn { "warn" } else { "ok" },
        "checks": checks
    }))
}

fn check_platform(checks: &mut Vec<Value>, platform: &PlatformSpec, base: &Path) {
    let prefix = platform.name;
    let instructions = base.join(platform.instructions);
    match fs::read_to_string(&instructions) {
        Ok(content) if content.contains(POLICY_MARKER_V3) => checks.push(check(
            format!("{prefix}.policy"),
            "ok",
            format!("v3 policy in {}", instructions.display()),
            None,
        )),
        Ok(content) if content.contains(POLICY_MARKER_V2) => checks.push(check(
            format!("{prefix}.policy"),
            "warn",
            format!("v2 policy in {}", instructions.display()),
            Some("run `mem setup <platform>` to upgrade the policy block to v3"),
        )),
        Ok(content) if content.contains("mnemark memory policy") => checks.push(check(
            format!("{prefix}.policy"),
            "warn",
            format!("legacy policy block in {}", instructions.display()),
            Some("run `mem setup <platform>` and replace the old block with v3"),
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
        let skill_md = base.join(skills_dir).join("mnemark/SKILL.md");
        if skill_md.exists() {
            checks.push(check(
                format!("{prefix}.skill"),
                "ok",
                format!("skill installed at {}", skill_md.display()),
                None,
            ));
        } else {
            checks.push(check(
                format!("{prefix}.skill"),
                "missing",
                format!("{} not found", skill_md.display()),
                Some("run `mem setup <platform>`"),
            ));
        }
    }

    if let Some(settings_rel) = platform.claude_settings {
        let settings = base.join(settings_rel);
        let status = fs::read_to_string(&settings)
            .ok()
            .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
            .map(|root| {
                let present = root
                    .pointer("/hooks/SessionStart")
                    .and_then(Value::as_array)
                    .map(|entries| {
                        entries.iter().any(|entry| {
                            entry
                                .get("hooks")
                                .and_then(Value::as_array)
                                .map(|hooks| {
                                    hooks.iter().any(|hook| {
                                        hook.get("command")
                                            .and_then(Value::as_str)
                                            .map(|command| {
                                                command.contains("mem prime")
                                                    || command.contains("session-prime")
                                            })
                                            .unwrap_or(false)
                                    })
                                })
                                .unwrap_or(false)
                        })
                    })
                    .unwrap_or(false);
                if present {
                    "present"
                } else {
                    "absent"
                }
            });
        match status {
            Some("present") => checks.push(check(
                format!("{prefix}.session_hook"),
                "ok",
                format!("SessionStart hook in {}", settings.display()),
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

fn check(id: impl Into<String>, status: &str, detail: String, fix: Option<&str>) -> Value {
    let mut entry = json!({
        "id": id.into(),
        "status": status,
        "detail": detail
    });
    if let Some(fix) = fix {
        entry["fix"] = json!(fix);
    }
    entry
}
