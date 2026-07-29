use super::super::*;
use super::listing::workflow_scope_filter;

fn artifact_report(
    app: &App,
    content: &str,
    args: &WorkflowValidateArgs,
) -> Result<Option<workflow_core::WorkflowArtifactReport>> {
    if !args.check_artifacts {
        return Ok(None);
    }
    workflow_core::validate_artifact_references(content, &app.root, args.repo.as_deref()).map(Some)
}

fn add_quality_warnings(result: &mut Value, content: &str) {
    let mut warnings = Vec::new();
    if workflow_core::post_run_memory(content).is_empty() {
        warnings.push(json!({
            "code": "no_post_run_memory",
            "hint": "add a post_run_memory section so every execution ends with a save-learnings step"
        }));
    }
    if workflow_core::outputs(content).is_empty() {
        warnings.push(json!({
            "code": "no_outputs",
            "hint": "add outputs so the checklist has observable completion criteria"
        }));
    }
    if !warnings.is_empty() {
        result["warnings"] = Value::Array(warnings);
    }
}

pub(super) fn validate_file(app: &App, args: WorkflowValidateArgs) -> Result<()> {
    let path = args
        .file
        .as_deref()
        .ok_or_else(|| mem_core::error::usage("workflow validate --file requires a file path"))?;
    let content = required_content(None, Some(path))?;
    workflow_core::validate_content(&content)?;
    let artifact_report = artifact_report(app, &content, &args)?;
    let mut result = json!({
        "status": "valid",
        "source": "file",
        "path": path.display().to_string(),
        "scope_and_tags_checked": false,
        "artifact_checks": artifact_report
    });
    add_quality_warnings(&mut result, &content);
    print_json_pretty(&result)
}

pub(super) fn validate_stored(
    app: &App,
    conn: &Connection,
    args: WorkflowValidateArgs,
) -> Result<()> {
    let reference = args.reference.as_deref().ok_or_else(|| {
        mem_core::error::usage("workflow validate requires a reference or --file <FILE>")
    })?;
    let scope_filter = workflow_scope_filter(Some(&args.scope))?;
    let scope_refs = scope_filter
        .as_ref()
        .map(|scopes| scopes.iter().map(String::as_str).collect::<Vec<_>>());
    let workflow = workflow_by_ref_in_scopes(conn, reference, scope_refs.as_deref())?;
    workflow_core::validate_record(&workflow)?;
    let content = workflow.content.as_deref().unwrap_or_default();
    let artifact_report = artifact_report(app, content, &args)?;
    let mut result = json!({
        "status": "valid",
        "source": "store",
        "id": workflow.id,
        "name": workflow.name,
        "scope_and_tags_checked": true,
        "artifact_checks": artifact_report
    });
    add_quality_warnings(&mut result, content);
    print_json_pretty(&result)
}
