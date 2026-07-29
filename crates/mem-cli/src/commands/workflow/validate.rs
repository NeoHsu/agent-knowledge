use super::super::*;
use super::listing::workflow_scope_filter;

pub(super) fn validate(app: &App, conn: &Connection, args: WorkflowValidateArgs) -> Result<()> {
    let scope_filter = workflow_scope_filter(Some(&args.scope))?;
    let scope_refs = scope_filter
        .as_ref()
        .map(|scopes| scopes.iter().map(String::as_str).collect::<Vec<_>>());
    let workflow = workflow_by_ref_in_scopes(conn, &args.reference, scope_refs.as_deref())?;
    workflow_core::validate_record(&workflow)?;
    let artifact_report = if args.check_artifacts {
        Some(workflow_core::validate_artifact_references(
            workflow.content.as_deref().unwrap_or_default(),
            &app.root,
        )?)
    } else {
        None
    };
    let mut result = json!({
        "status": "valid",
        "id": workflow.id,
        "name": workflow.name,
        "artifact_checks": artifact_report
    });
    if workflow_core::post_run_memory(workflow.content.as_deref().unwrap_or_default()).is_empty() {
        result["warnings"] = json!([{
            "code": "no_post_run_memory",
            "hint": "add a post_run_memory section so every execution ends with a save-learnings step"
        }]);
    }
    print_json_pretty(&result)
}
