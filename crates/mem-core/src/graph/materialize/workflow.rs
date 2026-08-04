use std::collections::HashMap;

use anyhow::Result;
use rusqlite::Connection;
use serde_json::json;
use serde_yaml::Value as YamlValue;

use crate::db::Memory;

use super::super::DETERMINISTIC;
use super::super::ids::{artifact_node_id, memory_node_id, safe_node_part, workflow_step_node_id};
use super::super::model::GraphNode;
use super::super::store::{insert_node, insert_simple_node};
use super::support::{insert_artifact_node, insert_edge_simple, memory_is_active};

pub(super) fn add_workflow_edges(conn: &Connection, memory: &Memory) -> Result<()> {
    let Some(content) = memory.content.as_deref() else {
        return Ok(());
    };
    let Ok(value) = serde_yaml::from_str::<YamlValue>(content) else {
        return Ok(());
    };
    let Some(mapping) = value.as_mapping() else {
        return Ok(());
    };
    let memory_node_id = memory_node_id(&memory.id);

    if let Some(reusable_scripts) =
        yaml_get(mapping, "reusable_scripts").and_then(YamlValue::as_sequence)
    {
        for (index, script) in reusable_scripts.iter().enumerate() {
            let Some(script) = script.as_mapping() else {
                continue;
            };
            let Some(path) = yaml_get(script, "path").and_then(YamlValue::as_str) else {
                continue;
            };
            let owner = yaml_get(script, "owner")
                .and_then(YamlValue::as_str)
                .unwrap_or_default();
            let required = yaml_get(script, "required")
                .and_then(YamlValue::as_bool)
                .unwrap_or(false);
            let artifact_id = artifact_node_id(path);
            insert_artifact_node(
                conn,
                path,
                Some(&memory.scope),
                json!({"owner": owner, "required": required, "workflow": memory.id}),
            )?;
            insert_edge_simple(
                conn,
                &memory_node_id,
                &artifact_id,
                "references_artifact",
                "workflow reusable_scripts entry",
                Some(&memory.id),
                Some(&memory.scope),
                1.1,
                DETERMINISTIC,
                json!({"owner": owner, "required": required, "index": index}),
            )?;
        }
    }

    if let Some(steps) = yaml_get(mapping, "steps").and_then(YamlValue::as_sequence) {
        for (index, step) in steps.iter().enumerate() {
            let Some(step) = step.as_mapping() else {
                continue;
            };
            let step_id = yaml_get(step, "id")
                .and_then(YamlValue::as_str)
                .filter(|id| !id.trim().is_empty())
                .map(str::to_string)
                .unwrap_or_else(|| format!("step_{}", index + 1));
            let step_node_id = workflow_step_node_id(&memory.id, &step_id);
            let run = yaml_get(step, "run")
                .and_then(YamlValue::as_str)
                .map(str::to_string);
            let confirm = yaml_get(step, "confirm")
                .and_then(YamlValue::as_bool)
                .unwrap_or(false);
            insert_node(
                conn,
                &GraphNode {
                    id: step_node_id.clone(),
                    kind: "workflow_step".to_string(),
                    label: step_id.clone(),
                    ref_table: Some("memories".to_string()),
                    ref_id: Some(memory.id.clone()),
                    scope: Some(memory.scope.clone()),
                    metadata: json!({
                        "workflow": memory.id,
                        "step_id": step_id,
                        "index": index,
                        "run": run,
                        "confirm": confirm,
                    }),
                    origin: DETERMINISTIC.to_string(),
                },
            )?;
            insert_edge_simple(
                conn,
                &memory_node_id,
                &step_node_id,
                "has_workflow_step",
                "workflow steps entry",
                Some(&memory.id),
                Some(&memory.scope),
                1.0,
                DETERMINISTIC,
                json!({"index": index}),
            )?;
            let run_artifact = yaml_get(step, "run")
                .and_then(YamlValue::as_str)
                .and_then(|run| first_artifact_token(run).map(|path| (run, path)));
            if let Some((run, path)) = run_artifact {
                let artifact_id = artifact_node_id(&path);
                insert_artifact_node(
                    conn,
                    &path,
                    Some(&memory.scope),
                    json!({"workflow": memory.id}),
                )?;
                insert_edge_simple(
                    conn,
                    &step_node_id,
                    &artifact_id,
                    "step_uses_artifact",
                    "workflow step run starts with artifact path",
                    Some(&memory.id),
                    Some(&memory.scope),
                    1.1,
                    DETERMINISTIC,
                    json!({"run": run}),
                )?;
            }
            if confirm {
                let concept_id = "concept:confirmation_required".to_string();
                insert_simple_node(
                    conn,
                    &concept_id,
                    "concept",
                    "confirmation_required",
                    None,
                    DETERMINISTIC,
                    json!({}),
                )?;
                insert_edge_simple(
                    conn,
                    &step_node_id,
                    &concept_id,
                    "requires_confirmation",
                    "workflow step confirm flag is true",
                    Some(&memory.id),
                    Some(&memory.scope),
                    1.0,
                    DETERMINISTIC,
                    json!({}),
                )?;
            }
        }
    }

    Ok(())
}

pub(super) fn add_workflow_run_edges(
    conn: &Connection,
    memory_index: &HashMap<String, Memory>,
) -> Result<()> {
    let mut stmt = conn.prepare(
        "SELECT id, memory_id, result, note, source, created_at
         FROM workflow_runs
         ORDER BY id",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, Option<String>>(3)?,
            row.get::<_, Option<String>>(4)?,
            row.get::<_, String>(5)?,
        ))
    })?;
    for row in rows {
        let (id, memory_id, result, note, source, created_at) = row?;
        let Some(workflow) = memory_index
            .get(&memory_id)
            .filter(|memory| memory_is_active(memory))
        else {
            continue;
        };
        let scope = Some(workflow.scope.clone());
        let workflow_label = workflow.name.clone();
        let run_id = format!("workflow_run:{id}");
        insert_node(
            conn,
            &GraphNode {
                id: run_id.clone(),
                kind: "workflow_run".to_string(),
                label: format!("{workflow_label} run {id}: {result}"),
                ref_table: Some("workflow_runs".to_string()),
                ref_id: Some(id.to_string()),
                scope: scope.clone(),
                metadata: json!({
                    "memory_id": memory_id,
                    "result": result,
                    "note": note,
                    "source": source,
                    "created_at": created_at,
                }),
                origin: DETERMINISTIC.to_string(),
            },
        )?;
        let memory_node = memory_node_id(&memory_id);
        insert_edge_simple(
            conn,
            &memory_node,
            &run_id,
            "recorded_run",
            "workflow run history record",
            Some(&memory_id),
            scope.as_deref(),
            1.0,
            DETERMINISTIC,
            json!({}),
        )?;
        let result_concept = format!("concept:run_{}", safe_node_part(&result));
        insert_simple_node(
            conn,
            &result_concept,
            "concept",
            &format!("run_{result}"),
            None,
            DETERMINISTIC,
            json!({}),
        )?;
        insert_edge_simple(
            conn,
            &run_id,
            &result_concept,
            "has_result",
            "workflow run result",
            Some(&memory_id),
            scope.as_deref(),
            1.0,
            DETERMINISTIC,
            json!({}),
        )?;
    }
    Ok(())
}

fn yaml_get<'a>(mapping: &'a serde_yaml::Mapping, key: &str) -> Option<&'a YamlValue> {
    mapping.get(YamlValue::String(key.to_string()))
}

fn first_artifact_token(run: &str) -> Option<String> {
    let token = run.split_whitespace().next()?.trim_matches(['"', '\'']);
    token.starts_with("artifacts/").then(|| token.to_string())
}
