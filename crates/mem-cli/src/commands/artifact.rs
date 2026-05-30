use anyhow::{bail, Result};

use crate::args::{ArtifactCommand, ArtifactKindArg};
use crate::commands::print_json_pretty;
use mem_core::app::App;
use mem_core::artifact::{
    add_artifact, check_artifacts, remove_artifact, update_artifact_checksum, AddArtifact,
    ArtifactKind, ArtifactManifest,
};
use mem_core::util::parse_string_array;

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
            let tags = args.tags.as_deref().map(parse_string_array).transpose()?;
            let entry = add_artifact(
                &app.root,
                AddArtifact {
                    path: &args.path,
                    name: args.name,
                    kind: artifact_kind(args.kind),
                    scope: args.scope,
                    description: args.description,
                    executable: args.executable,
                    tags,
                    force: args.force,
                },
            )?;
            print_json_pretty(&entry)
        }
        ArtifactCommand::Update(args) => {
            if !args.checksum {
                bail!("artifact update currently requires --checksum");
            }
            let entry = update_artifact_checksum(&app.root, &args.name)?;
            print_json_pretty(&entry)
        }
        ArtifactCommand::Remove(args) => {
            let entry = remove_artifact(&app.root, &args.name, args.delete_file)?;
            print_json_pretty(&entry)
        }
    }
}

fn artifact_kind(kind: ArtifactKindArg) -> ArtifactKind {
    match kind {
        ArtifactKindArg::Script => ArtifactKind::Script,
        ArtifactKindArg::Template => ArtifactKind::Template,
        ArtifactKindArg::Snippet => ArtifactKind::Snippet,
        ArtifactKindArg::Reference => ArtifactKind::Reference,
    }
}
