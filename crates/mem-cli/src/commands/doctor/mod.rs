mod platform;
mod report;
mod store;

use super::*;
use crate::commands::setup::{
    PLATFORMS, PlatformSpec, SHARED_SKILLS_DIR, base_dir, platform_by_name,
};
use platform::{check_platform, check_shared_skill};
use report::check;

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
    store::check_store(&mut checks, app)?;

    let base = base_dir(args.base_dir.as_deref());
    let shared_skill_root = base.join(SHARED_SKILLS_DIR).join("mnemark");
    check_shared_skill(&mut checks, &shared_skill_root);
    let platforms: Vec<&PlatformSpec> = match args.platform.as_deref() {
        Some(name) => {
            vec![platform_by_name(name).ok_or_else(|| anyhow!("unknown platform: {name}"))?]
        }
        None => PLATFORMS.iter().collect(),
    };
    for platform in platforms {
        check_platform(&mut checks, platform, &base, &shared_skill_root);
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
