use anyhow::{Result, bail};

use crate::args::{ArtifactCommand, ArtifactKindArg};
use crate::commands::print_json_pretty;
use mem_core::app::App;
use mem_core::artifact::{
    AddArtifact, ArtifactKind, ArtifactManifest, add_artifact, check_artifacts, remove_artifact,
    update_artifact_checksum, validate_artifact_file, validate_artifact_path,
};
use mem_core::scope;
use mem_core::util::{parse_string_array, sanitize_secret_field, sanitize_secret_file};

pub(crate) fn cmd_artifact(app: &App, command: ArtifactCommand) -> Result<()> {
    match command {
        ArtifactCommand::List => {
            let entries = ArtifactManifest::load(&app.root)?
                .map(|manifest| manifest.entries())
                .unwrap_or_default();
            print_json_pretty(&entries)
        }
        ArtifactCommand::Check => {
            let report = check_artifacts(&app.root)?;
            print_json_pretty(&report)
        }
        ArtifactCommand::Show(args) => {
            let Some(manifest) = ArtifactManifest::load(&app.root)? else {
                bail!("artifact manifest not found");
            };
            let entry = manifest.find_entry(&args.name)?;
            print_json_pretty(&entry)
        }
        ArtifactCommand::Add(args) => {
            let path_value = args.path.to_string_lossy();
            validate_artifact_path(&path_value).map_err(anyhow::Error::msg)?;
            sanitize_secret_field(&path_value, "artifact path", false)?;
            let name = args
                .name
                .as_deref()
                .map(|value| sanitize_secret_field(value, "artifact name", args.redact_secrets))
                .transpose()?;
            if name
                .as_deref()
                .is_some_and(|value| value.len() > 256 || value.chars().any(char::is_control))
            {
                bail!("artifact name exceeds 256 bytes or contains control characters");
            }
            let scope = sanitize_secret_field(&args.scope, "artifact scope", args.redact_secrets)?;
            let scope = scope::resolve_write_scope(&scope)?;
            let description = args
                .description
                .as_deref()
                .map(|value| {
                    sanitize_secret_field(value, "artifact description", args.redact_secrets)
                })
                .transpose()?;
            if description
                .as_deref()
                .is_some_and(|value| value.len() > 65_536)
            {
                bail!("artifact description exceeds 65536 bytes");
            }
            let tags = args
                .tags
                .as_deref()
                .map(parse_string_array)
                .transpose()?
                .map(|tags| {
                    tags.into_iter()
                        .map(|tag| sanitize_secret_field(&tag, "artifact tag", args.redact_secrets))
                        .collect::<Result<Vec<_>>>()
                })
                .transpose()?;
            if tags.as_ref().is_some_and(|tags| {
                tags.len() > 100
                    || tags.iter().map(String::len).sum::<usize>() > 65_536
                    || tags
                        .iter()
                        .any(|tag| tag.len() > 1_024 || tag.chars().any(char::is_control))
            }) {
                bail!("artifact tags exceed resource limits");
            }
            let artifact_file = validate_artifact_file(&app.root, &path_value)?;
            sanitize_secret_file(&artifact_file, "artifact file", args.redact_secrets)?;
            let entry = add_artifact(
                &app.root,
                AddArtifact {
                    path: &args.path,
                    name,
                    kind: artifact_kind(args.kind),
                    scope,
                    description,
                    executable: args.executable,
                    tags,
                    force: args.force,
                },
            )?;
            mark_graph_dirty_if_store(app)?;
            print_json_pretty(&entry)
        }
        ArtifactCommand::Update(args) => {
            if !args.checksum {
                bail!("artifact update currently requires --checksum");
            }
            let manifest = ArtifactManifest::load(&app.root)?
                .ok_or_else(|| anyhow::anyhow!("artifact manifest not found"))?;
            let existing = manifest.find_entry(&args.name)?;
            validate_artifact_path(&existing.record.path).map_err(anyhow::Error::msg)?;
            let artifact_file = validate_artifact_file(&app.root, &existing.record.path)?;
            sanitize_secret_file(&artifact_file, "artifact file", args.redact_secrets)?;
            let entry = update_artifact_checksum(&app.root, &args.name)?;
            mark_graph_dirty_if_store(app)?;
            print_json_pretty(&entry)
        }
        ArtifactCommand::Remove(args) => {
            let entry = remove_artifact(&app.root, &args.name, args.delete_file)?;
            mark_graph_dirty_if_store(app)?;
            print_json_pretty(&entry)
        }
    }
}

fn mark_graph_dirty_if_store(app: &App) -> Result<()> {
    if app.db_path.exists() {
        app.require_schema()?;
        let conn = app.conn()?;
        mem_core::graph::set_graph_dirty(&conn, true)?;
    }
    Ok(())
}

fn artifact_kind(kind: ArtifactKindArg) -> ArtifactKind {
    match kind {
        ArtifactKindArg::Script => ArtifactKind::Script,
        ArtifactKindArg::Template => ArtifactKind::Template,
        ArtifactKindArg::Snippet => ArtifactKind::Snippet,
        ArtifactKindArg::Reference => ArtifactKind::Reference,
    }
}
