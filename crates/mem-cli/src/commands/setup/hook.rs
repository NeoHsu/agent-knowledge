use super::super::*;

pub(crate) const LEGACY_SILENT_HOOK_COMMAND: &str = "mem prime 2>/dev/null || true";
pub(crate) const LEGACY_HOOK_COMMAND: &str = "mem prime 2>&1 || { status=$?; printf '\\n[mnemark unavailable: mem prime failed with exit %s; continue without memory reads or writes]\\n' \"$status\"; }";
pub(crate) const HOOK_COMMAND: &str = concat!(
    "mem --json-errors contract --skill-version ",
    env!("CARGO_PKG_VERSION"),
    " >/dev/null && mem --read-only prime 2>&1 || { status=$?; printf '\\n[mnemark unavailable: compatibility or read-only prime failed with exit %s; continue without memory reads or writes]\\n' \"$status\"; }",
);

pub(crate) fn is_legacy_hook_command(command: &str) -> bool {
    matches!(command, LEGACY_SILENT_HOOK_COMMAND | LEGACY_HOOK_COMMAND)
}

pub(crate) fn is_custom_prime_hook(command: &str) -> bool {
    command.contains("mem prime")
        || command.contains("mem --read-only prime")
        || command.contains("session-prime")
}

pub(super) fn wire_claude_hook(settings_path: &Path, dry_run: bool) -> Result<Value> {
    let mut root: Value = if settings_path.exists() {
        let raw = fs::read_to_string(settings_path)
            .with_context(|| format!("read {}", settings_path.display()))?;
        serde_json::from_str(&raw).with_context(|| {
            format!(
                "parse {}; fix the JSON before wiring the hook",
                settings_path.display()
            )
        })?
    } else {
        json!({})
    };
    let target_text = settings_path.display().to_string();

    let entries = root
        .as_object_mut()
        .ok_or_else(|| anyhow!("{target_text} is not a JSON object"))?
        .entry("hooks")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .ok_or_else(|| anyhow!("{target_text}: hooks is not a JSON object"))?
        .entry("SessionStart")
        .or_insert_with(|| json!([]))
        .as_array_mut()
        .ok_or_else(|| anyhow!("{target_text}: hooks.SessionStart is not a JSON array"))?;

    let mut upgraded = false;
    for entry in entries.iter_mut() {
        let Some(hooks) = entry.get_mut("hooks").and_then(Value::as_array_mut) else {
            continue;
        };
        for hook in hooks {
            let Some(command) = hook
                .get("command")
                .and_then(Value::as_str)
                .map(str::to_owned)
            else {
                continue;
            };
            if command == HOOK_COMMAND {
                return Ok(json!({"status": "already_present", "target": target_text}));
            }
            if is_legacy_hook_command(&command) {
                if dry_run {
                    return Ok(json!({
                        "status": "dry_run",
                        "action": "upgrade",
                        "target": target_text,
                        "command": HOOK_COMMAND
                    }));
                }
                hook["command"] = json!(HOOK_COMMAND);
                upgraded = true;
                break;
            }
            if is_custom_prime_hook(&command) {
                return Ok(json!({"status": "already_present", "target": target_text}));
            }
        }
        if upgraded {
            break;
        }
    }
    if dry_run {
        return Ok(json!({
            "status": "dry_run",
            "action": "install",
            "target": target_text,
            "command": HOOK_COMMAND
        }));
    }

    if !upgraded {
        entries.push(json!({
            "hooks": [{
                "type": "command",
                "command": HOOK_COMMAND,
                "timeout": 15,
                "statusMessage": "Loading mnemark memory context..."
            }]
        }));
    }
    if let Some(parent) = settings_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("create parent directory {}", parent.display()))?;
    }
    let mut rendered = serde_json::to_string_pretty(&root)?;
    rendered.push('\n');
    atomic_write(settings_path, rendered.as_bytes())
        .with_context(|| format!("write {}", settings_path.display()))?;
    Ok(json!({
        "status": if upgraded { "upgraded" } else { "installed" },
        "target": target_text,
        "command": HOOK_COMMAND
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_both_managed_legacy_hooks() {
        assert!(is_legacy_hook_command(LEGACY_SILENT_HOOK_COMMAND));
        assert!(is_legacy_hook_command(LEGACY_HOOK_COMMAND));
        assert!(!is_legacy_hook_command(HOOK_COMMAND));
    }
}
