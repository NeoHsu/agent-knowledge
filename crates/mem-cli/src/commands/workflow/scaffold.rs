use std::path::PathBuf;

use super::super::*;

/// Runbook templates are embedded so `workflow new` works without a source
/// checkout and always matches the binary's validation rules.
const MINIMAL_WORKFLOW_TEMPLATE: &str = include_str!("../../../../../templates/workflow.yaml");
const FULL_WORKFLOW_TEMPLATE: &str = include_str!("../../../../../templates/workflow-full.yaml");

pub(super) fn scaffold(args: WorkflowNewArgs) -> Result<()> {
    let WorkflowNewArgs {
        name,
        output,
        examples,
        force,
    } = args;
    let output = output.unwrap_or_else(|| PathBuf::from(format!("{name}.yaml")));
    if output.exists() && !force {
        bail!(
            "{} already exists; pass --force to overwrite",
            output.display()
        );
    }
    if let Some(parent) = output.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)
            .with_context(|| format!("create directory {}", parent.display()))?;
    }
    let (template_name, template) = match examples {
        WorkflowExamples::Minimal => ("minimal", MINIMAL_WORKFLOW_TEMPLATE),
        WorkflowExamples::Full => ("full", FULL_WORKFLOW_TEMPLATE),
    };
    atomic_write(&output, template.as_bytes())
        .with_context(|| format!("write {}", output.display()))?;

    let path = output.display().to_string();
    let tags = json!([format!("workflow:{name}")]).to_string();
    let edit_instruction = match examples {
        WorkflowExamples::Minimal => "replace every <replace: ...> value, then set draft: false",
        WorkflowExamples::Full => {
            "replace every <replace: ...> value, delete unused examples, then set draft: false"
        }
    };
    print_json_pretty(&json!({
        "status": "scaffolded",
        "path": path,
        "template": template_name,
        "draft": true,
        "next_steps": [
            edit_instruction,
            "run commands.validate_file; save only after validation succeeds",
            "run commands.save, then commands.validate_stored; run commands.validate_references when reusable_scripts are present"
        ],
        "commands": {
            "validate_file": ["mem", "workflow", "validate", "--file", path],
            "save": [
                "mem", "save", "--type", "workflow", "--name", name,
                "--tags", tags, "--content-file", path
            ],
            "validate_stored": ["mem", "workflow", "validate", name],
            "validate_references": [
                "mem", "workflow", "validate", name,
                "--check-artifacts", "--repo", "."
            ]
        }
    }))
}
