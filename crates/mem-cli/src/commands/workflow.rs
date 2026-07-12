use super::*;
use std::path::PathBuf;

/// Baseline runbook template, embedded so `workflow new` works without a
/// source checkout and always matches the binary's validation rules.
const WORKFLOW_TEMPLATE: &str = include_str!("../../../../templates/workflow.yaml");

pub(crate) fn cmd_workflow(app: &App, command: WorkflowCommand) -> Result<()> {
    app.require_schema()?;
    let writes_store = matches!(&command, WorkflowCommand::Record(_))
        || matches!(&command, WorkflowCommand::Show(args) if args.with_graph_context);
    let conn = if writes_store {
        app.conn()?
    } else {
        app.read_conn()?
    };
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
            let scope_filter = workflow_scope_filter(Some(&args.scope))?;
            let scope_refs = scope_filter
                .as_ref()
                .map(|scopes| scopes.iter().map(String::as_str).collect::<Vec<_>>());
            let workflow =
                workflow_by_ref_in_scopes(&conn, &args.reference, scope_refs.as_deref())?;
            let graph_context = if args.with_graph_context {
                mem_core::graph::ensure_fresh(&conn, &app.root)?;
                let scopes = if workflow.scope == "global" {
                    vec!["global".to_string()]
                } else {
                    vec!["global".to_string(), workflow.scope.clone()]
                };
                Some(mem_core::graph::explain(
                    &conn,
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
                            context.node.id,
                            neighbor.relation,
                            neighbor.confidence,
                            neighbor.node.id
                        ));
                    }
                }
                print_text(checklist)?;
            } else if let Some(context) = graph_context {
                print_json_pretty(&json!({
                    "workflow": workflow,
                    "graph_context": context
                }))?;
            } else {
                print_json_pretty(&workflow)?;
            }
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
        WorkflowCommand::New(args) => {
            let output = args
                .output
                .unwrap_or_else(|| PathBuf::from(format!("{}.yaml", args.name)));
            if output.exists() && !args.force {
                bail!(
                    "{} already exists; pass --force to overwrite",
                    output.display()
                );
            }
            if let Some(parent) = output.parent() {
                if !parent.as_os_str().is_empty() {
                    fs::create_dir_all(parent)
                        .with_context(|| format!("create directory {}", parent.display()))?;
                }
            }
            fs::write(&output, WORKFLOW_TEMPLATE)
                .with_context(|| format!("write {}", output.display()))?;
            print_json_pretty(&json!({
                "status": "scaffolded",
                "path": output.display().to_string(),
                "next_steps": [
                    "edit the YAML: fill goal, triggers, steps, stop_conditions; delete unused example steps",
                    format!(
                        "mem save --type workflow --name {} --tags '[\"workflow:{}\",\"intent:<intent>\"]' --content-file {}",
                        args.name, args.name, output.display()
                    ),
                    format!("mem workflow validate {}", args.name)
                ]
            }))?;
        }
        WorkflowCommand::Record(args) => {
            let scope_filter = workflow_scope_filter(Some(&args.scope))?;
            let scope_refs = scope_filter
                .as_ref()
                .map(|scopes| scopes.iter().map(String::as_str).collect::<Vec<_>>());
            let workflow =
                workflow_by_ref_in_scopes(&conn, &args.reference, scope_refs.as_deref())?;
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
            with_transaction(&conn, |conn| {
                log_workflow_run(
                    conn,
                    &workflow.id,
                    &args.result,
                    note.as_deref(),
                    &args.source,
                )?;
                mem_core::graph::set_graph_dirty(conn, true)
            })?;
            let (runs, failures) = workflow_run_counts(&conn, &workflow.id)?;
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
            // Feed the learning checklist to the agent at the moment the run
            // ends, so closing the loop does not depend on it remembering to.
            let post_run =
                workflow_core::post_run_memory(workflow.content.as_deref().unwrap_or_default());
            if post_run.is_empty() {
                response["post_run_memory_missing"] = json!(
                    "runbook has no post_run_memory section; add one with `mem update` \
                     so every run ends with a save-learnings check"
                );
            } else {
                response["post_run_memory"] = json!(post_run);
            }
            print_json_pretty(&response)?;
        }
        WorkflowCommand::Validate(args) => {
            let scope_filter = workflow_scope_filter(Some(&args.scope))?;
            let scope_refs = scope_filter
                .as_ref()
                .map(|scopes| scopes.iter().map(String::as_str).collect::<Vec<_>>());
            let workflow =
                workflow_by_ref_in_scopes(&conn, &args.reference, scope_refs.as_deref())?;
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
            if workflow_core::post_run_memory(workflow.content.as_deref().unwrap_or_default())
                .is_empty()
            {
                result["warnings"] = json!([{
                    "code": "no_post_run_memory",
                    "hint": "add a post_run_memory section so every execution ends with a save-learnings step"
                }]);
            }
            print_json_pretty(&result)?;
        }
    }
    Ok(())
}

fn workflow_scope_filter(scope: Option<&str>) -> Result<Option<Vec<String>>> {
    match scope {
        Some("auto") => Ok(Some(scope::detect_scope_set()?)),
        Some("all") | None => Ok(None),
        Some(value) => {
            scope::validate_scope(value)?;
            Ok(Some(vec!["global".to_string(), value.to_string()]))
        }
    }
}
