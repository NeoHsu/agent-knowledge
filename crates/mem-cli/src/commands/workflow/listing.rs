use super::super::*;

pub(super) fn list(app: &App, conn: &Connection, args: WorkflowListArgs) -> Result<()> {
    let limit = args
        .limit
        .or_else(|| app.config.workflow_default_limit())
        .unwrap_or(DEFAULT_LIMIT);
    let scope = args
        .scope
        .as_deref()
        .or_else(|| app.config.workflow_default_scope());
    let scope_filter = workflow_scope_filter(scope)?;
    let mut workflows = all_workflows(conn, args.include_superseded)?;
    workflow_core::retain_scope(&mut workflows, scope_filter.as_deref());
    workflows.truncate(limit);
    print_json_pretty(&workflows)
}

pub(super) fn find(app: &App, conn: &Connection, args: WorkflowFindArgs) -> Result<()> {
    let limit = args
        .limit
        .or_else(|| app.config.workflow_default_limit())
        .unwrap_or(DEFAULT_LIMIT);
    let scope = args
        .scope
        .as_deref()
        .or_else(|| app.config.workflow_default_scope());
    let scope_filter = workflow_scope_filter(scope)?;
    let mut workflows = all_workflows(conn, false)?;
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
    print_json_pretty(&workflows)
}

pub(super) fn workflow_scope_filter(scope: Option<&str>) -> Result<Option<Vec<String>>> {
    match scope {
        Some("auto") => Ok(Some(scope::detect_scope_set()?)),
        Some("all") | None => Ok(None),
        Some(value) => {
            scope::validate_scope(value)?;
            Ok(Some(vec!["global".to_string(), value.to_string()]))
        }
    }
}
