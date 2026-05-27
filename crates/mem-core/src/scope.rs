use anyhow::Result;

use crate::util::remote_to_scope;

pub fn detect_scope_set() -> Result<Vec<String>> {
    Ok(vec!["global".to_string(), detect_scope()?])
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
