use super::*;

pub(crate) fn cmd_workflow(app: &App, command: WorkflowCommand) -> Result<()> {
    app.init()?;
    let conn = app.conn()?;
    match command {
        WorkflowCommand::List(args) => {
            let scope_filter = workflow_scope_filter(args.scope.as_deref())?;
            let mut workflows = all_workflows(&conn, args.include_superseded)?;
            workflow_core::retain_scope(&mut workflows, scope_filter.as_deref());
            workflows.truncate(args.limit);
            print_json_pretty(&workflows)?;
        }
        WorkflowCommand::Show(args) => {
            let workflow = workflow_by_ref(&conn, &args.reference)?;
            print_json_pretty(&workflow)?;
        }
        WorkflowCommand::Find(args) => {
            let scope_filter = workflow_scope_filter(args.scope.as_deref())?;
            let mut workflows = all_workflows(&conn, false)?;
            workflow_core::retain_scope(&mut workflows, scope_filter.as_deref());
            workflows.retain(|memory| workflow_core::matches_intent(memory, &args.intent));
            workflows.sort_by_key(|workflow| {
                std::cmp::Reverse(workflow_core::rank(
                    workflow,
                    &args.intent,
                    scope_filter.as_deref(),
                ))
            });
            workflows.truncate(args.limit);
            print_json_pretty(&workflows)?;
        }
        WorkflowCommand::Validate(args) => {
            let workflow = workflow_by_ref(&conn, &args.reference)?;
            workflow_core::validate_record(&workflow)?;
            print_json_pretty(&json!({
                "status": "valid",
                "id": workflow.id,
                "name": workflow.name
            }))?;
        }
    }
    Ok(())
}

fn workflow_scope_filter(scope: Option<&str>) -> Result<Option<Vec<String>>> {
    match scope {
        Some("auto") => Ok(Some(scope::detect_scope_set()?)),
        Some(scope) => Ok(Some(vec!["global".to_string(), scope.to_string()])),
        None => Ok(None),
    }
}
