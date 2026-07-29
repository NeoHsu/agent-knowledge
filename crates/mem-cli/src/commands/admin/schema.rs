use super::super::*;
use crate::cli_error::not_found_error;

pub(crate) struct SchemaDocument {
    pub(crate) name: &'static str,
    pub(crate) version: u64,
    pub(crate) description: &'static str,
    source: &'static str,
}

pub(crate) const SCHEMAS: &[SchemaDocument] = &[
    SchemaDocument {
        name: "bundle-manifest-v2",
        version: 2,
        description: "Portable bundle.json integrity manifest",
        source: include_str!("../../../../../docs/schemas/bundle-manifest-v2.schema.json"),
    },
    SchemaDocument {
        name: "contract-v1",
        version: 1,
        description: "Store-independent mem contract response",
        source: include_str!("../../../../../docs/schemas/contract-v1.schema.json"),
    },
    SchemaDocument {
        name: "error-v1",
        version: 1,
        description: "Versioned JSON error envelope emitted with --json-errors",
        source: include_str!("../../../../../docs/schemas/error-v1.schema.json"),
    },
    SchemaDocument {
        name: "graph-export-v1",
        version: 1,
        description: "Deterministic graph export document",
        source: include_str!("../../../../../docs/schemas/graph-export-v1.schema.json"),
    },
    SchemaDocument {
        name: "memory-list-v1",
        version: 1,
        description: "JSON memory rows emitted by query and export",
        source: include_str!("../../../../../docs/schemas/memory-list-v1.schema.json"),
    },
    SchemaDocument {
        name: "operation-inspect-v1",
        version: 1,
        description: "Exact effects for one parsed mem invocation",
        source: include_str!("../../../../../docs/schemas/operation-inspect-v1.schema.json"),
    },
    SchemaDocument {
        name: "operation-list-v1",
        version: 1,
        description: "Stable CLI leaf-operation catalog",
        source: include_str!("../../../../../docs/schemas/operation-list-v1.schema.json"),
    },
    SchemaDocument {
        name: "prime-v1",
        version: 1,
        description: "Budgeted JSON session-prime response",
        source: include_str!("../../../../../docs/schemas/prime-v1.schema.json"),
    },
    SchemaDocument {
        name: "schema-list-v1",
        version: 1,
        description: "Bundled public schema catalog",
        source: include_str!("../../../../../docs/schemas/schema-list-v1.schema.json"),
    },
    SchemaDocument {
        name: "skill-compatibility-v1",
        version: 1,
        description: "Exact CLI and agent-skill compatibility manifest",
        source: include_str!("../../../../../docs/schemas/skill-compatibility-v1.schema.json"),
    },
];

pub(crate) fn schema_names() -> Vec<&'static str> {
    SCHEMAS.iter().map(|schema| schema.name).collect()
}

pub(crate) fn cmd_schema(command: SchemaCommand) -> Result<()> {
    match command {
        SchemaCommand::List => print_json_pretty(&json!({
            "contract_version": CLI_OUTPUT_CONTRACT_VERSION,
            "schemas": SCHEMAS
                .iter()
                .map(|schema| json!({
                    "name": schema.name,
                    "version": schema.version,
                    "description": schema.description
                }))
                .collect::<Vec<_>>()
        })),
        SchemaCommand::Print(args) => {
            let requested = args.name.strip_suffix(".schema.json").unwrap_or(&args.name);
            let schema = SCHEMAS
                .iter()
                .find(|schema| schema.name == requested)
                .ok_or_else(|| not_found_error(format!("schema not found: {}", args.name)))?;
            let document: Value = serde_json::from_str(schema.source)
                .with_context(|| format!("parse bundled schema {}", schema.name))?;
            print_json_pretty(&document)
        }
    }
}
