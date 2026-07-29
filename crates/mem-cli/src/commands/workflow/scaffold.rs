use std::path::PathBuf;

use super::super::*;

/// Baseline runbook template, embedded so `workflow new` works without a
/// source checkout and always matches the binary's validation rules.
const WORKFLOW_TEMPLATE: &str = include_str!("../../../../../templates/workflow.yaml");

pub(super) fn scaffold(args: WorkflowNewArgs) -> Result<()> {
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
    fs::write(&output, WORKFLOW_TEMPLATE).with_context(|| format!("write {}", output.display()))?;
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
    }))
}
