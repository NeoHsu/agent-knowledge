use super::super::*;
use super::listing::workflow_scope_filter;

pub(super) fn show(app: &App, conn: &Connection, args: WorkflowShowArgs) -> Result<()> {
    let scope_filter = workflow_scope_filter(Some(&args.scope))?;
    let scope_refs = scope_filter
        .as_ref()
        .map(|scopes| scopes.iter().map(String::as_str).collect::<Vec<_>>());
    let workflow = workflow_by_ref_in_scopes(conn, &args.reference, scope_refs.as_deref())?;
    let graph_context = if args.with_graph_context {
        mem_core::graph::ensure_fresh(conn, &app.root)?;
        let scopes = if workflow.scope == "global" {
            vec!["global".to_string()]
        } else {
            vec!["global".to_string(), workflow.scope.clone()]
        };
        Some(mem_core::graph::explain(
            conn,
            &workflow.id,
            1,
            Some(&scopes),
        )?)
    } else {
        None
    };
    if args.checklist {
        let mut checklist = workflow_core::render_checklist(&workflow)?;
        if let Some(context) = graph_context {
            checklist.push_str("\n[graph-context]\n");
            for neighbor in context.neighbors.iter().take(12) {
                checklist.push_str(&format!(
                    "- {} --{} [{}]-- {}\n",
                    context.node.id, neighbor.relation, neighbor.confidence, neighbor.node.id
                ));
            }
        }
        print_text(checklist)
    } else if let Some(context) = graph_context {
        print_json_pretty(&json!({
            "workflow": workflow,
            "graph_context": context
        }))
    } else {
        print_json_pretty(&workflow)
    }
}
