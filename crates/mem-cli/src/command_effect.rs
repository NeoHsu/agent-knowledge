use crate::args::{
    AmbiguityCommand, ArtifactCommand, BundleCommand, Command, GraphCommand, SetupCommand,
    WorkflowCommand,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StoreAccess {
    None,
    ReadOnly,
    SharedLock,
    ExclusiveLock,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NetworkEffect {
    None,
    Fetch,
    Push,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CommandEffect {
    pub(crate) store_access: StoreAccess,
    pub(crate) durable_write: bool,
    pub(crate) rebuildable_write: bool,
    pub(crate) output_file_write: bool,
    pub(crate) network: NetworkEffect,
}

impl CommandEffect {
    const fn new(store_access: StoreAccess) -> Self {
        Self {
            store_access,
            durable_write: false,
            rebuildable_write: false,
            output_file_write: false,
            network: NetworkEffect::None,
        }
    }

    const fn durable(mut self) -> Self {
        self.durable_write = true;
        self
    }

    const fn rebuildable(mut self) -> Self {
        self.rebuildable_write = true;
        self
    }

    const fn output_file(mut self) -> Self {
        self.output_file_write = true;
        self
    }

    const fn network(mut self, network: NetworkEffect) -> Self {
        self.network = network;
        self
    }

    pub(crate) fn classify(command: &Command, store_exists: bool) -> Self {
        let read_only = || Self::new(StoreAccess::ReadOnly);
        let exclusive = || Self::new(StoreAccess::ExclusiveLock);

        match command {
            Command::Init => exclusive().durable().rebuildable(),
            Command::Migrate(args) if args.dry_run => read_only(),
            Command::Migrate(_) => exclusive().durable().rebuildable(),
            Command::Save(_) | Command::Update(_) | Command::Supersede(_) | Command::Delete(_) => {
                exclusive().durable().rebuildable()
            }
            Command::Query(args) if args.touch && !args.no_touch && args.repair_index => {
                exclusive().durable().rebuildable()
            }
            Command::Query(args) if args.touch && !args.no_touch => exclusive().durable(),
            Command::Query(args) if args.repair_index => exclusive().rebuildable(),
            Command::Query(_) => read_only(),
            Command::Prime(args) if args.focus.is_some() && store_exists => {
                exclusive().rebuildable()
            }
            Command::Prime(_) | Command::Doctor(_) => read_only(),
            Command::Sync(args) if args.dry_run => read_only(),
            Command::Sync(args) => {
                exclusive()
                    .durable()
                    .rebuildable()
                    .network(if args.push && !args.no_push {
                        NetworkEffect::Push
                    } else {
                        NetworkEffect::Fetch
                    })
            }
            Command::Reindex => exclusive().rebuildable(),
            Command::Context(_) | Command::Config { .. } => Self::new(StoreAccess::None),
            Command::Contract => Self::new(StoreAccess::None),
            Command::Setup { command } => {
                let writes_files = match command {
                    SetupCommand::List => false,
                    SetupCommand::ClaudeCode(args)
                    | SetupCommand::Codex(args)
                    | SetupCommand::Pi(args)
                    | SetupCommand::GeminiCli(args)
                    | SetupCommand::Opencode(args) => !args.dry_run,
                };
                if writes_files {
                    Self::new(StoreAccess::None).output_file()
                } else {
                    Self::new(StoreAccess::None)
                }
            }
            Command::History(_) | Command::Stats(_) => read_only(),
            Command::Audit(args) if args.fix => exclusive().durable().rebuildable(),
            Command::Audit(_) | Command::Reconcile(_) | Command::Export(_) => read_only(),
            Command::Gc(_) | Command::Import(_) | Command::Merge(_) => {
                exclusive().durable().rebuildable()
            }
            Command::Bundle { command } => match command {
                BundleCommand::Inspect(_) => Self::new(StoreAccess::None),
                BundleCommand::Export(_) => Self::new(StoreAccess::SharedLock).output_file(),
                BundleCommand::Import(_) => exclusive().durable().rebuildable(),
            },
            Command::Retro { .. } => read_only(),
            Command::Workflow { command } => match command {
                WorkflowCommand::Record(_) => exclusive().durable().rebuildable(),
                WorkflowCommand::Show(args) if args.with_graph_context => exclusive().rebuildable(),
                WorkflowCommand::New(_) => read_only().output_file(),
                WorkflowCommand::List(_)
                | WorkflowCommand::Show(_)
                | WorkflowCommand::Find(_)
                | WorkflowCommand::Validate(_) => read_only(),
            },
            Command::Artifact { command } => match command {
                ArtifactCommand::Add(_)
                | ArtifactCommand::Update(_)
                | ArtifactCommand::Remove(_) => exclusive().durable().rebuildable(),
                ArtifactCommand::List | ArtifactCommand::Check | ArtifactCommand::Show(_) => {
                    read_only()
                }
            },
            Command::Ambiguity { command } => match command {
                AmbiguityCommand::List(_) => read_only(),
                AmbiguityCommand::Add(_) | AmbiguityCommand::Resolve(_) => {
                    exclusive().durable().rebuildable()
                }
            },
            Command::Graph { command } => match command {
                GraphCommand::Stats | GraphCommand::Review(_) => read_only(),
                GraphCommand::Candidates(args) if !args.unlinked => read_only(),
                GraphCommand::Ingest(_) | GraphCommand::Accept(_) | GraphCommand::Reject(_) => {
                    exclusive().durable().rebuildable()
                }
                GraphCommand::Rebuild
                | GraphCommand::Explain(_)
                | GraphCommand::Path(_)
                | GraphCommand::Query(_)
                | GraphCommand::Export(_)
                | GraphCommand::Candidates(_) => exclusive().rebuildable(),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::args::Cli;
    use clap::Parser;

    fn effect(args: &[&str], store_exists: bool) -> CommandEffect {
        let cli = Cli::try_parse_from(args)
            .unwrap_or_else(|error| panic!("parse {}: {error}", args.join(" ")));
        CommandEffect::classify(&cli.command, store_exists)
    }

    #[test]
    fn classifies_conditional_store_and_network_effects() {
        let none = CommandEffect::new(StoreAccess::None);
        let read = CommandEffect::new(StoreAccess::ReadOnly);
        let exclusive = CommandEffect::new(StoreAccess::ExclusiveLock);

        let cases: &[(&[&str], bool, CommandEffect)] = &[
            (&["mem", "contract"], false, none),
            (&["mem", "context", "--detect"], false, none),
            (&["mem", "config", "show"], false, none),
            (&["mem", "setup", "list"], false, none),
            (&["mem", "setup", "pi", "--dry-run"], false, none),
            (&["mem", "setup", "pi"], false, none.output_file()),
            (&["mem", "migrate", "--dry-run"], true, read),
            (&["mem", "query", "term"], true, read),
            (
                &["mem", "query", "term", "--touch"],
                true,
                exclusive.durable(),
            ),
            (
                &["mem", "query", "term", "--repair-index"],
                true,
                exclusive.rebuildable(),
            ),
            (
                &["mem", "query", "term", "--touch", "--repair-index"],
                true,
                exclusive.durable().rebuildable(),
            ),
            (&["mem", "prime", "--focus", "task"], false, read),
            (
                &["mem", "prime", "--focus", "task"],
                true,
                exclusive.rebuildable(),
            ),
            (&["mem", "sync", "--dry-run"], true, read),
            (
                &["mem", "sync"],
                true,
                exclusive
                    .durable()
                    .rebuildable()
                    .network(NetworkEffect::Fetch),
            ),
            (
                &["mem", "sync", "--push"],
                true,
                exclusive
                    .durable()
                    .rebuildable()
                    .network(NetworkEffect::Push),
            ),
            (
                &["mem", "bundle", "export", "store.tgz"],
                true,
                CommandEffect::new(StoreAccess::SharedLock).output_file(),
            ),
            (&["mem", "bundle", "inspect", "store.tgz"], false, none),
            (
                &["mem", "workflow", "show", "release", "--with-graph-context"],
                true,
                exclusive.rebuildable(),
            ),
            (
                &["mem", "workflow", "new", "release"],
                true,
                read.output_file(),
            ),
            (&["mem", "artifact", "list"], true, read),
            (
                &["mem", "artifact", "remove", "helper"],
                true,
                exclusive.durable().rebuildable(),
            ),
            (&["mem", "ambiguity", "list"], true, read),
            (&["mem", "graph", "stats"], true, read),
            (&["mem", "graph", "review"], true, read),
            (&["mem", "graph", "candidates"], true, read),
            (
                &["mem", "graph", "candidates", "--unlinked"],
                true,
                exclusive.rebuildable(),
            ),
            (
                &["mem", "graph", "ingest", "edges.json"],
                true,
                exclusive.durable().rebuildable(),
            ),
        ];

        for (args, store_exists, expected) in cases {
            assert_eq!(effect(args, *store_exists), *expected, "{}", args.join(" "));
        }
    }

    #[test]
    fn every_top_level_command_family_has_an_effect_case() {
        let cases: &[&[&str]] = &[
            &["mem", "init"],
            &["mem", "migrate"],
            &["mem", "save", "--name", "x", "--content", "x"],
            &["mem", "query"],
            &["mem", "prime"],
            &["mem", "doctor"],
            &["mem", "sync"],
            &["mem", "update", "x", "--content", "x"],
            &["mem", "supersede", "old", "new", "--content", "x"],
            &["mem", "delete", "x"],
            &["mem", "reindex"],
            &["mem", "context", "--detect"],
            &["mem", "config", "show"],
            &["mem", "contract"],
            &["mem", "setup", "list"],
            &["mem", "history"],
            &["mem", "stats"],
            &["mem", "audit"],
            &["mem", "reconcile"],
            &["mem", "gc"],
            &["mem", "export"],
            &["mem", "import", "memories.json"],
            &["mem", "merge", "theirs.db"],
            &["mem", "bundle", "inspect", "store.tgz"],
            &["mem", "retro", "daily"],
            &["mem", "workflow", "list"],
            &["mem", "artifact", "list"],
            &["mem", "ambiguity", "list"],
            &["mem", "graph", "stats"],
        ];

        for args in cases {
            let _ = effect(args, true);
        }
    }

    #[test]
    fn runtime_model_documents_conditional_effect_cases() {
        let runtime_model = include_str!("../../../docs/runtime-model.md");
        for required in [
            "`query --touch`",
            "`query --repair-index`",
            "`prime --focus`",
            "`sync --dry-run`",
            "`sync --push`",
            "`graph candidates --unlinked`",
            "`workflow show --with-graph-context`",
        ] {
            assert!(
                runtime_model.contains(required),
                "runtime effect matrix must document {required}"
            );
        }
    }
}
