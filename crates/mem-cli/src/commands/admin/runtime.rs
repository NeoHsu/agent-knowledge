use super::super::*;
use mem_core::config::user_config_path;

pub(crate) fn cmd_context(args: ContextArgs) -> Result<()> {
    if !args.detect {
        bail!("missing required action. Try `mem context --detect` to show the detected project scope, or `mem context --help` for options.");
    }
    print_json(&json!({"scope": scope::detect_scope()?}))?;
    Ok(())
}

pub(crate) fn cmd_config(app: &App, command: ConfigCommand) -> Result<()> {
    match command {
        ConfigCommand::Show => {
            print_json_pretty(&json!({
                "root": app.root.display().to_string(),
                "store_source": app.store_source.as_str(),
                "db_path": app.db_path.display().to_string(),
                "index_path": app.index_path.display().to_string(),
                "user_config_path": user_config_path().display().to_string(),
                "user_config_exists": user_config_path().exists(),
                "store_config_path": app.root.join("config.toml").display().to_string(),
                "store_config_exists": app.root.join("config.toml").exists(),
                "env": {
                    "MNEMARK_HOME": std::env::var("MNEMARK_HOME").ok(),
                    "XDG_CONFIG_HOME": std::env::var("XDG_CONFIG_HOME").ok()
                },
                "effective": {
                    "knowledge_home": app.root.display().to_string(),
                    "schema": "embedded",
                    "query_default_scope": app.config.query_default_scope(),
                    "query_default_limit": app.config.query_default_limit().unwrap_or(DEFAULT_LIMIT),
                    "query_candidate_limit": app.config.query_candidate_limit(),
                    "workflow_default_scope": app.config.workflow_default_scope(),
                    "workflow_default_limit": app.config.workflow_default_limit().unwrap_or(DEFAULT_LIMIT),
                    "budget_per_scope_max": app.config.per_scope_max()
                },
                "config": app.config
            }))?;
        }
    }
    Ok(())
}
