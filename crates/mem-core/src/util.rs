use std::collections::HashSet;
use std::fs;
use std::path::Path;

use anyhow::{anyhow, bail, Context, Result};
use chrono::{DateTime, Utc};
use regex::Regex;
use serde_json::{json, Value};

use crate::db::Memory;

pub fn confidence_for_source(source: &str) -> &'static str {
    match source {
        "manual" => "high",
        "agent" => "medium",
        "daily_retro" | "weekly_retro" => "low",
        _ => "medium",
    }
}

pub fn source_priority(source: &str) -> u8 {
    match source {
        "manual" => 4,
        "agent" => 3,
        "daily_retro" => 2,
        "weekly_retro" => 1,
        _ => 2,
    }
}

pub fn version_conflict(memory: &Memory, expected_version: i64) -> Option<Value> {
    (memory.version != expected_version).then(|| {
        json!({
            "status": "version_conflict",
            "id": memory.id,
            "name": memory.name,
            "expected_version": expected_version,
            "actual_version": memory.version
        })
    })
}

pub fn now() -> String {
    Utc::now().to_rfc3339()
}

pub fn strip_secrets(input: &str) -> Result<String> {
    let patterns = [
        r"sk-[A-Za-z0-9_\-]{16,}",
        r"ghp_[A-Za-z0-9_]{16,}",
        r"xoxb-[A-Za-z0-9\-]{16,}",
        r"AKIA[0-9A-Z]{16}",
        r"(?i)bearer\s+[A-Za-z0-9._\-]{16,}",
        r"(?i)(password|secret)\s*=\s*[^ \n\r]+",
    ];
    let mut output = input.to_string();
    for pattern in patterns {
        let re = Regex::new(pattern)?;
        output = re.replace_all(&output, "[REDACTED]").to_string();
    }
    Ok(output)
}

pub fn slugify(name: &str) -> String {
    let mut slug = String::new();
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch.to_ascii_lowercase());
        } else if (ch == '_' || ch == '-' || ch.is_whitespace() || ch == '/')
            && !slug.ends_with('_')
        {
            slug.push('_');
        }
    }
    let slug = slug.trim_matches('_').to_string();
    if slug.is_empty() {
        format!("memory_{}", uuid::Uuid::new_v4())
    } else {
        slug
    }
}

pub fn required_content(content: Option<String>, content_file: Option<&Path>) -> Result<String> {
    optional_content(content, content_file)?
        .ok_or_else(|| anyhow!("one of --content or --content-file is required"))
}

pub fn optional_content(
    content: Option<String>,
    content_file: Option<&Path>,
) -> Result<Option<String>> {
    match (content, content_file) {
        (Some(_), Some(_)) => bail!("use only one of --content or --content-file"),
        (Some(content), None) => Ok(Some(content)),
        (None, Some(path)) => fs::read_to_string(path)
            .with_context(|| format!("read {}", path.display()))
            .map(Some),
        (None, None) => Ok(None),
    }
}

pub fn validate_tags(tags: &str) -> Result<()> {
    parse_string_array(tags)?;
    Ok(())
}

pub fn parse_string_array(raw: &str) -> Result<Vec<String>> {
    let parsed: Value =
        serde_json::from_str(raw).context("tags/memory_ids must be a JSON array")?;
    let Value::Array(values) = parsed else {
        bail!("expected JSON array");
    };
    values
        .into_iter()
        .map(|value| {
            value
                .as_str()
                .map(str::to_string)
                .ok_or_else(|| anyhow!("array items must be strings"))
        })
        .collect()
}

pub fn memory_has_tag(tags: &str, wanted: &str) -> bool {
    parse_string_array(tags)
        .map(|tags| tags.iter().any(|tag| tag == wanted))
        .unwrap_or(false)
}

pub fn merge_tags(existing: &str, add: &str) -> Result<String> {
    let mut set = HashSet::new();
    for raw in [existing, add] {
        let Value::Array(values) = serde_json::from_str(raw)? else {
            bail!("tags must be JSON arrays");
        };
        for value in values {
            if let Some(tag) = value.as_str() {
                set.insert(tag.to_string());
            }
        }
    }
    let mut tags: Vec<_> = set.into_iter().collect();
    tags.sort();
    Ok(serde_json::to_string(&tags)?)
}

pub fn is_expired(expires_at: Option<&str>) -> bool {
    expires_at
        .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
        .map(|expires| expires.with_timezone(&Utc) < Utc::now())
        .unwrap_or(false)
}

pub fn content_similarity(a: &str, b: &str) -> f64 {
    let a_set = shingles(&normalized_text(a));
    let b_set = shingles(&normalized_text(b));
    if a_set.is_empty() || b_set.is_empty() {
        return 0.0;
    }
    let intersection = a_set.intersection(&b_set).count() as f64;
    let union = a_set.union(&b_set).count() as f64;
    let smaller = a_set.len().min(b_set.len()) as f64;
    (intersection / union).max(intersection / smaller)
}

pub fn normalized_text(input: &str) -> String {
    input
        .chars()
        .flat_map(char::to_lowercase)
        .filter(|ch| !ch.is_whitespace())
        .collect()
}

pub fn remote_to_scope(remote: &str) -> String {
    let cleaned = remote
        .trim_end_matches(".git")
        .replace("git@github.com:", "")
        .replace("https://github.com/", "");
    if cleaned.contains('/') {
        format!("project:{cleaned}")
    } else {
        "global".to_string()
    }
}

fn shingles(input: &str) -> HashSet<String> {
    let chars: Vec<char> = input.chars().collect();
    if chars.is_empty() {
        return HashSet::new();
    }
    if chars.len() <= 2 {
        return HashSet::from([input.to_string()]);
    }
    chars
        .windows(2)
        .map(|window| window.iter().collect::<String>())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn memory_with_version(version: i64) -> Memory {
        Memory {
            id: "id".to_string(),
            r#type: "feedback".to_string(),
            name: "name".to_string(),
            description: None,
            content: Some("content".to_string()),
            tags: "[]".to_string(),
            scope: "global".to_string(),
            source: "manual".to_string(),
            confidence: "high".to_string(),
            protected: true,
            created_at: now(),
            updated_at: now(),
            expires_at: None,
            valid_until: None,
            superseded_by: None,
            version,
            access_count: 0,
            last_accessed_at: None,
        }
    }

    #[test]
    fn source_priority_orders_trust() {
        assert!(source_priority("manual") > source_priority("agent"));
        assert!(source_priority("agent") > source_priority("daily_retro"));
        assert!(source_priority("daily_retro") > source_priority("weekly_retro"));
    }

    #[test]
    fn version_conflict_reports_mismatch() {
        let memory = memory_with_version(3);
        assert!(version_conflict(&memory, 3).is_none());
        let conflict = version_conflict(&memory, 2).expect("conflict");
        assert_eq!(conflict["status"], "version_conflict");
        assert_eq!(conflict["actual_version"], 3);
    }

    #[test]
    fn strips_common_secret_patterns() {
        let stripped = strip_secrets("token=Bearer abcdefghijklmnop password=hunter2").unwrap();
        assert!(stripped.contains("[REDACTED]"));
        assert!(!stripped.contains("hunter2"));
    }

    #[test]
    fn parses_string_arrays_only() {
        assert_eq!(parse_string_array(r#"["a","b"]"#).unwrap(), vec!["a", "b"]);
        assert!(parse_string_array(r#"["a",1]"#).is_err());
        assert!(parse_string_array(r#"{"a":1}"#).is_err());
    }

    #[test]
    fn content_similarity_handles_cjk_overlap() {
        let score = content_similarity("不要使用 emoji", "不要在回覆中使用 emoji");
        assert!(score > 0.8);
    }

    #[test]
    fn remote_scope_supports_ssh_and_https() {
        assert_eq!(
            remote_to_scope("git@github.com:NeoHsu/agent-knowledge.git"),
            "project:NeoHsu/agent-knowledge"
        );
        assert_eq!(
            remote_to_scope("https://github.com/NeoHsu/agent-knowledge.git"),
            "project:NeoHsu/agent-knowledge"
        );
    }
}
