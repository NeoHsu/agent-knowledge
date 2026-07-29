use super::super::*;
use super::schema::schema_names;
use crate::cli_error::version_mismatch_error;

fn skill_update_command() -> String {
    format!(
        "npx skills add https://github.com/NeoHsu/mnemark/tree/v{} --skill mnemark -y",
        env!("CARGO_PKG_VERSION")
    )
}

pub(crate) fn cmd_contract(args: ContractArgs) -> Result<()> {
    let skill_compatibility = args.skill_version.as_deref().map(|skill_version| {
        let compatible = skill_version == env!("CARGO_PKG_VERSION");
        json!({
            "cli_version": env!("CARGO_PKG_VERSION"),
            "skill_version": skill_version,
            "target_version": env!("CARGO_PKG_VERSION"),
            "compatible": compatible,
            "recommended_action": if compatible {
                "proceed"
            } else {
                "install the skill release matching the CLI, then rerun this gate"
            },
            "update_command": if compatible {
                Value::Null
            } else {
                Value::String(skill_update_command())
            }
        })
    });

    print_json_pretty(&json!({
        "status": "ok",
        "contract_version": CLI_OUTPUT_CONTRACT_VERSION,
        "cli_version": env!("CARGO_PKG_VERSION"),
        "compatibility": {
            "successful_json": "required fields remain compatible within a minor release; additive fields are allowed",
            "json_errors": "versioned by contract_version and emitted only with --json-errors",
            "pre_1_0_breaking_changes": "may occur only in a documented minor release"
        },
        "json_errors": {
            "version": CLI_OUTPUT_CONTRACT_VERSION,
            "required_fields": ["status", "contract_version", "code", "message", "exit_code", "retryable"],
            "optional_fields": ["details"],
            "known_codes": [
                "cli_parse_error",
                "command_failed",
                "compatibility",
                "conflict",
                "index_stale_after_write",
                "integrity",
                "not_found",
                "safety_violation",
                "usage",
                "version_mismatch"
            ]
        },
        "schemas": {
            "store": supported_schema_version(),
            "bundle": super::super::bundle::BUNDLE_FORMAT_VERSION,
            "workflow": mem_core::workflow::WORKFLOW_SCHEMA_VERSION,
            "graph": mem_core::graph::GRAPH_SCHEMA_VERSION,
            "benchmark_report": BENCHMARK_REPORT_CONTRACT_VERSION
        },
        "published_schemas": schema_names(),
        "skill_compatibility": skill_compatibility
    }))?;

    if let Some(skill_version) = args.skill_version.as_deref() {
        if skill_version != env!("CARGO_PKG_VERSION") {
            let update_command = skill_update_command();
            return Err(version_mismatch_error(skill_version, &update_command));
        }
    }
    Ok(())
}
