use super::*;

pub(crate) fn cmd_workflow(app: &App, command: WorkflowCommand) -> Result<()> {
    app.ensure_schema()?;
    let conn = app.conn()?;
    match command {
        WorkflowCommand::List(args) => {
            let limit = args
                .limit
                .or_else(|| app.config.workflow_default_limit())
                .unwrap_or(DEFAULT_LIMIT);
            let scope = args
                .scope
                .as_deref()
                .or_else(|| app.config.workflow_default_scope());
            let scope_filter = workflow_scope_filter(scope)?;
            let mut workflows = all_workflows(&conn, args.include_superseded)?;
            workflow_core::retain_scope(&mut workflows, scope_filter.as_deref());
            workflows.truncate(limit);
            print_json_pretty(&workflows)?;
        }
        WorkflowCommand::Show(args) => {
            let workflow = workflow_by_ref(&conn, &args.reference)?;
            print_json_pretty(&workflow)?;
        }
        WorkflowCommand::Find(args) => {
            let limit = args
                .limit
                .or_else(|| app.config.workflow_default_limit())
                .unwrap_or(DEFAULT_LIMIT);
            let scope = args
                .scope
                .as_deref()
                .or_else(|| app.config.workflow_default_scope());
            let scope_filter = workflow_scope_filter(scope)?;
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
            workflows.truncate(limit);
            print_json_pretty(&workflows)?;
        }
        WorkflowCommand::Validate(args) => {
            let workflow = workflow_by_ref(&conn, &args.reference)?;
            workflow_core::validate_record(&workflow)?;
            let artifact_report = if args.check_artifacts {
                Some(workflow_core::validate_artifact_references(
                    workflow.content.as_deref().unwrap_or_default(),
                    &app.root,
                )?)
            } else {
                None
            };
            print_json_pretty(&json!({
                "status": "valid",
                "id": workflow.id,
                "name": workflow.name,
                "artifact_checks": artifact_report
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
