use anyhow::Result;

use crate::error;
use crate::util::remote_to_scope;

pub fn detect_scope_set() -> Result<Vec<String>> {
    Ok(vec!["global".to_string(), detect_scope()?])
}

pub fn resolve_write_scope(value: &str) -> Result<String> {
    let scope = if value == "auto" {
        detect_scope()?
    } else {
        value.to_string()
    };
    validate_scope(&scope)?;
    Ok(scope)
}

pub fn validate_scope(scope: &str) -> Result<()> {
    if scope == "global" {
        return Ok(());
    }
    let Some(project) = scope.strip_prefix("project:") else {
        return Err(error::usage(format!(
            "invalid scope {scope:?}; expected global or project:<owner/repo>"
        )));
    };
    let mut parts = project.split('/');
    let owner = parts.next().unwrap_or_default();
    let repo = parts.next().unwrap_or_default();
    let valid_component = |value: &str| {
        !value.is_empty()
            && value != "."
            && value != ".."
            && value.len() <= 100
            && value
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'))
    };
    if scope.len() > 256
        || !valid_component(owner)
        || !valid_component(repo)
        || parts.next().is_some()
    {
        return Err(error::usage(format!(
            "invalid scope {scope:?}; expected project:<owner/repo>"
        )));
    }
    Ok(())
}

pub fn detect_scope() -> Result<String> {
    let output = std::process::Command::new("git")
        .args(["remote", "get-url", "origin"])
        .output();
    let Ok(output) = output else {
        return Ok("global".to_string());
    };
    if !output.status.success() {
        return Ok("global".to_string());
    }
    let remote = String::from_utf8_lossy(&output.stdout);
    Ok(remote_to_scope(remote.trim()))
}
