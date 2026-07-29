use super::super::*;
use super::listing::workflow_scope_filter;

pub(super) fn record(conn: &Connection, args: WorkflowRecordArgs) -> Result<()> {
    let scope_filter = workflow_scope_filter(Some(&args.scope))?;
    let scope_refs = scope_filter
        .as_ref()
        .map(|scopes| scopes.iter().map(String::as_str).collect::<Vec<_>>());
    let workflow = workflow_by_ref_in_scopes(conn, &args.reference, scope_refs.as_deref())?;
    if workflow.r#type != "workflow" {
        bail!("memory is not a workflow: {}", workflow.name);
    }
    if args.source == "manual" && !args.user_confirmed {
        bail!("source=manual requires --user-confirmed");
    }
    if args
        .note
        .as_deref()
        .is_some_and(|value| value.len() > 65_536)
    {
        bail!("workflow run note exceeds 65536 bytes");
    }
    let note = args
        .note
        .as_deref()
        .map(|value| sanitize_secret_field(value, "workflow run note", args.redact_secrets))
        .transpose()?;
    with_transaction(conn, |conn| {
        log_workflow_run(
            conn,
            &workflow.id,
            &args.result,
            note.as_deref(),
            &args.source,
        )?;
        mem_core::graph::set_graph_dirty(conn, true)
    })?;
    let (runs, failures) = workflow_run_counts(conn, &workflow.id)?;
    let mut response = json!({
        "status": "recorded",
        "id": workflow.id,
        "name": workflow.name,
        "result": args.result,
        "runs_total": runs,
        "failures_total": failures
    });
    if args.result == "failure" {
        response["hint"] = json!(
            "save the durable lesson with `mem save`, and update the runbook \
             with `mem update` if a step is stale"
        );
    }
    // Feed the learning checklist to the agent at the moment the run ends, so
    // closing the loop does not depend on it remembering to.
    let post_run = workflow_core::post_run_memory(workflow.content.as_deref().unwrap_or_default());
    if post_run.is_empty() {
        response["post_run_memory_missing"] = json!(
            "runbook has no post_run_memory section; add one with `mem update` \
             so every run ends with a save-learnings check"
        );
    } else {
        response["post_run_memory"] = json!(post_run);
    }
    print_json_pretty(&response)
}
