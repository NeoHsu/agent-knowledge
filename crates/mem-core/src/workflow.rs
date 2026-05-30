use std::collections::BTreeSet;
use std::path::Path;

use anyhow::{anyhow, bail, Context, Result};
use serde::Serialize;
use serde_json::Value;
use serde_yaml::Value as YamlValue;

use crate::artifact::{
    artifact_file_checksum, artifact_file_is_executable, validate_artifact_path, ArtifactManifest,
};
use crate::{db::Memory, util::normalized_text};

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
        bail!("memory is not a workflow: {}", memory.name);
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

#[derive(Debug, Clone, Serialize)]
pub struct WorkflowArtifactReport {
    pub checked: usize,
    pub references: Vec<WorkflowArtifactReference>,
}

#[derive(Debug, Clone, Serialize)]
pub struct WorkflowArtifactReference {
    pub path: String,
    pub owner: String,
    pub required: bool,
    pub manifest_entry: Option<String>,
    pub checksum: Option<String>,
}

pub fn validate_artifact_references(
    content: &str,
    store_root: &Path,
) -> Result<WorkflowArtifactReport> {
    let value: YamlValue = serde_yaml::from_str(content).context("parse workflow YAML/JSON")?;
    let mapping = value
        .as_mapping()
        .ok_or_else(|| anyhow!("workflow content must be a YAML/JSON object"))?;
    let manifest = ArtifactManifest::load(store_root)?;
    let mut errors = Vec::new();
    let mut references = Vec::new();
    let mut declared_knowledge_store_paths = BTreeSet::new();

    if let Some(reusable_scripts) = yaml_get(mapping, "reusable_scripts") {
        let scripts = reusable_scripts
            .as_sequence()
            .ok_or_else(|| anyhow!("workflow reusable_scripts must be an array"))?;
        for (index, script) in scripts.iter().enumerate() {
            let Some(script) = script.as_mapping() else {
                errors.push(format!("reusable_scripts[{index}] must be an object"));
                continue;
            };
            let path = yaml_get(script, "path")
                .and_then(YamlValue::as_str)
                .unwrap_or_default();
            let owner = yaml_get(script, "owner")
                .and_then(YamlValue::as_str)
                .unwrap_or_default();
            let required = match yaml_get(script, "required") {
                Some(YamlValue::Bool(value)) => *value,
                Some(_) => {
                    errors.push(format!(
                        "reusable_scripts[{index}] required must be boolean"
                    ));
                    false
                }
                None => false,
            };
            if path.trim().is_empty() {
                errors.push(format!("reusable_scripts[{index}] requires path"));
                continue;
            }
            if !matches!(owner, "repo" | "knowledge_store") {
                errors.push(format!(
                    "reusable_scripts[{index}] owner must be repo or knowledge_store"
                ));
                continue;
            }
            if owner == "repo" {
                references.push(WorkflowArtifactReference {
                    path: path.to_string(),
                    owner: owner.to_string(),
                    required,
                    manifest_entry: None,
                    checksum: None,
                });
                continue;
            }

            declared_knowledge_store_paths.insert(path.to_string());
            validate_one_artifact_reference(
                index,
                script,
                path,
                owner,
                required,
                store_root,
                manifest.as_ref(),
                &mut errors,
                &mut references,
            )?;
        }
    }

    for (index, path) in step_artifact_runs(mapping)? {
        if !declared_knowledge_store_paths.contains(&path) {
            errors.push(format!(
                "workflow step {index} run references artifact path {path}, but reusable_scripts entry is missing"
            ));
        }
    }

    if !errors.is_empty() {
        bail!("workflow artifact validation failed: {}", errors.join("; "));
    }
    Ok(WorkflowArtifactReport {
        checked: references
            .iter()
            .filter(|reference| reference.owner == "knowledge_store")
            .count(),
        references,
    })
}

#[allow(clippy::too_many_arguments)]
fn validate_one_artifact_reference(
    index: usize,
    script: &serde_yaml::Mapping,
    path: &str,
    owner: &str,
    required: bool,
    store_root: &Path,
    manifest: Option<&ArtifactManifest>,
    errors: &mut Vec<String>,
    references: &mut Vec<WorkflowArtifactReference>,
) -> Result<()> {
    if let Err(reason) = validate_artifact_path(path) {
        errors.push(format!(
            "reusable_scripts[{index}] unsafe artifact path: {reason}"
        ));
        return Ok(());
    }
    let Some(manifest) = manifest else {
        errors.push(format!(
            "reusable_scripts[{index}] references {path}, but manifest.toml is missing"
        ));
        return Ok(());
    };
    let Some(entry) = manifest
        .entries()
        .into_iter()
        .find(|entry| entry.record.path == path)
    else {
        errors.push(format!(
            "reusable_scripts[{index}] references {path}, but manifest entry is missing"
        ));
        return Ok(());
    };
    let full_path = store_root.join(path);
    if !full_path.exists() {
        if required {
            errors.push(format!(
                "reusable_scripts[{index}] required artifact is missing: {path}"
            ));
        }
        references.push(WorkflowArtifactReference {
            path: path.to_string(),
            owner: owner.to_string(),
            required,
            manifest_entry: Some(entry.name),
            checksum: None,
        });
        return Ok(());
    }
    let actual_checksum = artifact_file_checksum(&full_path)?;
    if actual_checksum != entry.record.checksum {
        errors.push(format!(
            "reusable_scripts[{index}] checksum mismatch for {path}: expected {}, got {actual_checksum}",
            entry.record.checksum
        ));
    }
    if let Some(expected) = yaml_get(script, "checksum").and_then(YamlValue::as_str) {
        if expected != actual_checksum {
            errors.push(format!(
                "reusable_scripts[{index}] workflow checksum mismatch for {path}: expected {expected}, got {actual_checksum}"
            ));
        }
    }
    if entry.record.executable == Some(true) && !artifact_file_is_executable(&full_path)? {
        errors.push(format!(
            "reusable_scripts[{index}] artifact is not executable: {path}"
        ));
    }
    references.push(WorkflowArtifactReference {
        path: path.to_string(),
        owner: owner.to_string(),
        required,
        manifest_entry: Some(entry.name),
        checksum: Some(actual_checksum),
    });
    Ok(())
}

fn step_artifact_runs(mapping: &serde_yaml::Mapping) -> Result<Vec<(usize, String)>> {
    let Some(steps) = yaml_get(mapping, "steps").and_then(YamlValue::as_sequence) else {
        return Ok(Vec::new());
    };
    let mut paths = Vec::new();
    for (index, step) in steps.iter().enumerate() {
        let Some(step) = step.as_mapping() else {
            continue;
        };
        let Some(run) = yaml_get(step, "run").and_then(YamlValue::as_str) else {
            continue;
        };
        let Some(path) = first_artifact_token(run) else {
            continue;
        };
        if let Err(reason) = validate_artifact_path(&path) {
            bail!("workflow step {index} run has unsafe artifact path: {reason}");
        }
        paths.push((index, path));
    }
    Ok(paths)
}

fn first_artifact_token(run: &str) -> Option<String> {
    let token = run.split_whitespace().next()?.trim_matches(['"', '\'']);
    token.starts_with("artifacts/").then(|| token.to_string())
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
    if let Some(scopes) = scope_filter {
        if scopes.last().map(String::as_str) == Some(memory.scope.as_str())
            && memory.scope != "global"
        {
            score += 100;
        }
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
        bail!("workflow memory requires at least one workflow:* tag");
    }
    if scope.starts_with("project:") && !tags.iter().any(|tag| tag == scope) {
        bail!("project-scoped workflow requires matching {scope} tag");
    }
    Ok(())
}

fn validate_content(content: &str) -> Result<()> {
    let value: YamlValue = serde_yaml::from_str(content).context("parse workflow YAML/JSON")?;
    let mapping = value
        .as_mapping()
        .ok_or_else(|| anyhow!("workflow content must be a YAML/JSON object"))?;
    for field in [
        "schema_version",
        "goal",
        "triggers",
        "steps",
        "stop_conditions",
    ] {
        yaml_get(mapping, field)
            .ok_or_else(|| anyhow!("workflow missing required field: {field}"))?;
    }
    require_non_empty_sequence(mapping, "triggers")?;
    require_non_empty_sequence(mapping, "stop_conditions")?;
    let steps = yaml_get(mapping, "steps")
        .and_then(YamlValue::as_sequence)
        .ok_or_else(|| anyhow!("workflow steps must be a non-empty array"))?;
    if steps.is_empty() {
        bail!("workflow steps must be a non-empty array");
    }
    for (index, step) in steps.iter().enumerate() {
        let step_mapping = step
            .as_mapping()
            .ok_or_else(|| anyhow!("workflow step {index} must be an object"))?;
        let id = yaml_get(step_mapping, "id")
            .and_then(YamlValue::as_str)
            .unwrap_or_default();
        if id.trim().is_empty() {
            bail!("workflow step {index} requires non-empty id");
        }
        if !["run", "check", "manual", "ask"]
            .iter()
            .any(|key| yaml_get(step_mapping, key).is_some())
        {
            bail!("workflow step {id} requires one of run, check, manual, or ask");
        }
        if let Some(confirm) = yaml_get(step_mapping, "confirm") {
            if !matches!(confirm, YamlValue::Bool(_)) {
                bail!("workflow step {id} confirm must be boolean");
            }
        }
    }
    Ok(())
}

fn yaml_get<'a>(mapping: &'a serde_yaml::Mapping, key: &str) -> Option<&'a YamlValue> {
    mapping.get(YamlValue::String(key.to_string()))
}

fn require_non_empty_sequence(mapping: &serde_yaml::Mapping, field: &str) -> Result<()> {
    let sequence = yaml_get(mapping, field)
        .and_then(YamlValue::as_sequence)
        .ok_or_else(|| anyhow!("workflow {field} must be a non-empty array"))?;
    if sequence.is_empty() {
        bail!("workflow {field} must be a non-empty array");
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
    let value: Value = serde_json::from_str(raw).context("parse JSON array")?;
    let Some(array) = value.as_array() else {
        bail!("expected JSON array");
    };
    let mut strings = Vec::with_capacity(array.len());
    for item in array {
        let Some(value) = item.as_str() else {
            bail!("expected array of strings");
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
