use anyhow::Result;
use serde_json::Value;
use serde_yaml::Value as YamlValue;

use crate::{db::Memory, error, util::normalized_text};

mod artifacts;
mod checklist;

pub use artifacts::{
    WorkflowArtifactReference, WorkflowArtifactReport, validate_artifact_references,
};
pub use checklist::{outputs, post_run_memory, render_checklist};

#[cfg(test)]
use checklist::push_checklist_field;

pub const WORKFLOW_SCHEMA_VERSION: i64 = 1;

pub fn validate_memory(
    memory_type: &str,
    content: &str,
    tags: &str,
    scope: &str,
    no_validate_workflow: bool,
) -> Result<()> {
    if memory_type != "workflow" || no_validate_workflow {
        return Ok(());
    }
    validate_content(content)?;
    let tags = parse_string_array(tags)?;
    validate_tags(&tags, scope)?;
    Ok(())
}

pub fn validate_record(memory: &Memory) -> Result<()> {
    if memory.r#type != "workflow" {
        return Err(error::conflict(format!(
            "memory is not a workflow: {}",
            memory.name
        )));
    }
    validate_record_content(memory)
}

pub fn validate_record_content(memory: &Memory) -> Result<()> {
    validate_memory(
        &memory.r#type,
        memory.content.as_deref().unwrap_or_default(),
        &memory.tags,
        &memory.scope,
        false,
    )
}

pub fn retain_scope(memories: &mut Vec<Memory>, scope_filter: Option<&[String]>) {
    if let Some(scopes) = scope_filter {
        memories.retain(|memory| scopes.contains(&memory.scope));
    }
}

pub fn matches_intent(memory: &Memory, intent: &str) -> bool {
    let intent = intent.trim();
    if intent.is_empty() {
        return true;
    }
    let normalized_intent = normalized_text(intent);
    if normalized_intent.is_empty() {
        return true;
    }
    if has_intent_tag(memory, intent) {
        return true;
    }
    [
        memory.name.as_str(),
        memory.description.as_deref().unwrap_or_default(),
        memory.content.as_deref().unwrap_or_default(),
        memory.tags.as_str(),
    ]
    .iter()
    .any(|value| normalized_text(value).contains(&normalized_intent))
}

pub fn rank(memory: &Memory, intent: &str, scope_filter: Option<&[String]>) -> i64 {
    let mut score = 0;
    if let Some(scopes) = scope_filter
        && scopes.last().map(String::as_str) == Some(memory.scope.as_str())
        && memory.scope != "global"
    {
        score += 100;
    }
    score += match memory.confidence.as_str() {
        "high" => 30,
        "medium" => 20,
        _ => 10,
    };
    if has_intent_tag(memory, intent) {
        score += 50;
    }
    if name_matches_intent(memory, intent) {
        score += 35;
    }
    if tags_contain_intent_token(memory, intent) {
        score += 15;
    }
    score
}

fn validate_tags(tags: &[String], scope: &str) -> Result<()> {
    if !tags.iter().any(|tag| tag.starts_with("workflow:")) {
        return Err(error::usage(
            "workflow memory requires at least one workflow:* tag",
        ));
    }
    if scope.starts_with("project:") && !tags.iter().any(|tag| tag == scope) {
        return Err(error::usage(format!(
            "project-scoped workflow requires matching {scope} tag"
        )));
    }
    Ok(())
}

pub fn validate_content(content: &str) -> Result<()> {
    let value: YamlValue = serde_yaml::from_str(content)
        .map_err(|source| error::usage(format!("parse workflow YAML/JSON: {source}")))?;
    let mapping = value
        .as_mapping()
        .ok_or_else(|| error::usage("workflow content must be a YAML/JSON object"))?;
    for field in [
        "schema_version",
        "goal",
        "triggers",
        "steps",
        "stop_conditions",
    ] {
        yaml_get(mapping, field)
            .ok_or_else(|| error::usage(format!("workflow missing required field: {field}")))?;
    }
    let schema_version = yaml_get(mapping, "schema_version")
        .and_then(YamlValue::as_i64)
        .ok_or_else(|| error::usage("workflow schema_version must be an integer"))?;
    if schema_version != WORKFLOW_SCHEMA_VERSION {
        return Err(error::compatibility(format!(
            "unsupported workflow schema_version {schema_version}; expected {WORKFLOW_SCHEMA_VERSION}"
        )));
    }
    match yaml_get(mapping, "draft") {
        Some(YamlValue::Bool(true)) => {
            return Err(error::usage(
                "workflow is still a scaffold draft; replace placeholders and set draft: false before validation or save",
            ));
        }
        Some(YamlValue::Bool(false)) | None => {}
        Some(_) => return Err(error::usage("workflow draft must be boolean")),
    }
    if contains_scaffold_placeholder(&value) {
        return Err(error::usage(
            "workflow contains scaffold placeholders; replace every <replace: ...> value before validation or save",
        ));
    }
    require_non_empty_sequence(mapping, "triggers")?;
    require_non_empty_sequence(mapping, "stop_conditions")?;
    let steps = yaml_get(mapping, "steps")
        .and_then(YamlValue::as_sequence)
        .ok_or_else(|| error::usage("workflow steps must be a non-empty array"))?;
    if steps.is_empty() {
        return Err(error::usage("workflow steps must be a non-empty array"));
    }
    for (index, step) in steps.iter().enumerate() {
        let step_mapping = step
            .as_mapping()
            .ok_or_else(|| error::usage(format!("workflow step {index} must be an object")))?;
        let id = yaml_get(step_mapping, "id")
            .and_then(YamlValue::as_str)
            .unwrap_or_default();
        if id.trim().is_empty() {
            return Err(error::usage(format!(
                "workflow step {index} requires non-empty id"
            )));
        }
        if !["run", "check", "manual", "ask"]
            .iter()
            .any(|key| yaml_get(step_mapping, key).is_some())
        {
            return Err(error::usage(format!(
                "workflow step {id} requires one of run, check, manual, or ask"
            )));
        }
        if let Some(confirm) = yaml_get(step_mapping, "confirm")
            && !matches!(confirm, YamlValue::Bool(_))
        {
            return Err(error::usage(format!(
                "workflow step {id} confirm must be boolean"
            )));
        }
    }
    Ok(())
}

fn contains_scaffold_placeholder(value: &YamlValue) -> bool {
    if value
        .as_str()
        .is_some_and(|text| text.contains("<replace:"))
    {
        return true;
    }
    if let Some(items) = value.as_sequence() {
        return items.iter().any(contains_scaffold_placeholder);
    }
    value
        .as_mapping()
        .is_some_and(|mapping| mapping.values().any(contains_scaffold_placeholder))
}

fn yaml_get<'a>(mapping: &'a serde_yaml::Mapping, key: &str) -> Option<&'a YamlValue> {
    mapping.get(YamlValue::String(key.to_string()))
}

fn require_non_empty_sequence(mapping: &serde_yaml::Mapping, field: &str) -> Result<()> {
    let sequence = yaml_get(mapping, field)
        .and_then(YamlValue::as_sequence)
        .ok_or_else(|| error::usage(format!("workflow {field} must be a non-empty array")))?;
    if sequence.is_empty() {
        return Err(error::usage(format!(
            "workflow {field} must be a non-empty array"
        )));
    }
    Ok(())
}

fn has_intent_tag(memory: &Memory, intent: &str) -> bool {
    let exact_intent_tag = format!("intent:{}", intent.to_ascii_lowercase());
    let exact_workflow_tag = format!("workflow:{}", intent.to_ascii_lowercase());
    let normalized_intent = intent_key(intent);
    parse_string_array(&memory.tags)
        .map(|tags| {
            tags.iter().any(|tag| {
                let tag = tag.to_ascii_lowercase();
                if tag == exact_intent_tag || tag == exact_workflow_tag {
                    return true;
                }
                let Some((kind, value)) = tag.split_once(':') else {
                    return false;
                };
                matches!(kind, "intent" | "workflow") && intent_key(value) == normalized_intent
            })
        })
        .unwrap_or(false)
}

fn name_matches_intent(memory: &Memory, intent: &str) -> bool {
    let name_key = intent_key(&memory.name);
    let intent_key = intent_key(intent);
    !intent_key.is_empty() && (name_key == intent_key || name_key.contains(&intent_key))
}

fn tags_contain_intent_token(memory: &Memory, intent: &str) -> bool {
    let wanted_key = intent_key(intent);
    if wanted_key.is_empty() {
        return false;
    }
    parse_string_array(&memory.tags)
        .map(|tags| tags.iter().any(|tag| intent_key(tag).contains(&wanted_key)))
        .unwrap_or(false)
}

fn intent_key(input: &str) -> String {
    let mut key = String::new();
    for ch in input.chars() {
        if ch.is_ascii_alphanumeric() {
            key.push(ch.to_ascii_lowercase());
        } else if !key.ends_with('_') {
            key.push('_');
        }
    }
    key.trim_matches('_').to_string()
}

fn parse_string_array(raw: &str) -> Result<Vec<String>> {
    let value: Value = serde_json::from_str(raw)
        .map_err(|source| error::usage(format!("parse JSON array: {source}")))?;
    let Some(array) = value.as_array() else {
        return Err(error::usage("expected JSON array"));
    };
    let mut strings = Vec::with_capacity(array.len());
    for item in array {
        let Some(value) = item.as_str() else {
            return Err(error::usage("expected array of strings"));
        };
        strings.push(value.to_string());
    }
    Ok(strings)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Memory;

    #[test]
    fn checklist_fields_preserve_multiline_boundaries() {
        let mut output = String::new();
        push_checklist_field(&mut output, "ACTION (RUN)", "printf one\nprintf two");
        assert_eq!(
            output,
            "       ACTION (RUN):\n         | printf one\n         | printf two\n"
        );
    }

    #[test]
    fn intent_tags_match_normalized_separators() {
        let memory = Memory {
            id: "id".to_string(),
            r#type: "workflow".to_string(),
            name: "ci_triage_workflow".to_string(),
            description: None,
            content: Some("Triage CI failures".to_string()),
            tags: r#"["workflow:ci-triage","intent:fix-ci"]"#.to_string(),
            scope: "global".to_string(),
            source: "manual".to_string(),
            confidence: "high".to_string(),
            protected: true,
            created_at: "2026-05-27T00:00:00Z".to_string(),
            updated_at: "2026-05-27T00:00:00Z".to_string(),
            expires_at: None,
            valid_until: None,
            superseded_by: None,
            version: 1,
            access_count: 0,
            last_accessed_at: None,
            origin: "direct".to_string(),
            origin_ref: None,
            user_confirmed_at: Some("2026-05-27T00:00:00Z".to_string()),
        };

        assert!(matches_intent(&memory, "fix ci"));
        assert!(matches_intent(&memory, "ci_triage"));
    }

    #[test]
    fn rank_prefers_precise_intent_matches() {
        let broad = Memory {
            id: "broad".to_string(),
            r#type: "workflow".to_string(),
            name: "maintenance".to_string(),
            description: None,
            content: Some("General workflow that mentions deploy in a checklist".to_string()),
            tags: r#"["workflow:maintenance"]"#.to_string(),
            scope: "global".to_string(),
            source: "manual".to_string(),
            confidence: "high".to_string(),
            protected: true,
            created_at: "2026-05-27T00:00:00Z".to_string(),
            updated_at: "2026-05-27T00:00:00Z".to_string(),
            expires_at: None,
            valid_until: None,
            superseded_by: None,
            version: 1,
            access_count: 0,
            last_accessed_at: None,
            origin: "direct".to_string(),
            origin_ref: None,
            user_confirmed_at: Some("2026-05-27T00:00:00Z".to_string()),
        };
        let precise = Memory {
            id: "precise".to_string(),
            r#type: "workflow".to_string(),
            name: "deploy_workflow".to_string(),
            description: None,
            content: Some("Run deployment steps".to_string()),
            tags: r#"["workflow:deploy","intent:deploy"]"#.to_string(),
            source: "agent".to_string(),
            confidence: "medium".to_string(),
            ..broad.clone()
        };

        assert!(rank(&precise, "deploy", None) > rank(&broad, "deploy", None));
    }
}
