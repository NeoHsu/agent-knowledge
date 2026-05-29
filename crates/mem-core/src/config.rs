use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct Config {
    pub knowledge_home: Option<String>,
    pub default_scope: Option<String>,
    pub default_limit: Option<usize>,
    pub query: QueryConfig,
    pub workflow: WorkflowConfig,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct QueryConfig {
    pub default_scope: Option<String>,
    pub default_limit: Option<usize>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct WorkflowConfig {
    pub default_scope: Option<String>,
    pub default_limit: Option<usize>,
}

impl Config {
    pub fn load_user() -> Result<Self> {
        read_config(&user_config_path())
    }

    pub fn load_store(root: &Path) -> Result<Self> {
        read_config(&root.join("config.toml"))
    }

    pub fn merged_for_root(root: &Path, user: &Config) -> Result<Self> {
        let store = Self::load_store(root)?;
        Ok(store.overlay(user))
    }

    pub fn overlay(mut self, higher: &Config) -> Self {
        if higher.knowledge_home.is_some() {
            self.knowledge_home = higher.knowledge_home.clone();
        }
        if higher.default_scope.is_some() {
            self.default_scope = higher.default_scope.clone();
            self.query.default_scope = higher.default_scope.clone();
            self.workflow.default_scope = higher.default_scope.clone();
        }
        if higher.default_limit.is_some() {
            self.default_limit = higher.default_limit;
            self.query.default_limit = higher.default_limit;
            self.workflow.default_limit = higher.default_limit;
        }
        if higher.query.default_scope.is_some() {
            self.query.default_scope = higher.query.default_scope.clone();
        }
        if higher.query.default_limit.is_some() {
            self.query.default_limit = higher.query.default_limit;
        }
        if higher.workflow.default_scope.is_some() {
            self.workflow.default_scope = higher.workflow.default_scope.clone();
        }
        if higher.workflow.default_limit.is_some() {
            self.workflow.default_limit = higher.workflow.default_limit;
        }
        self
    }

    pub fn knowledge_home_path(&self) -> Option<PathBuf> {
        self.knowledge_home.as_deref().map(expand_home)
    }

    pub fn query_default_scope(&self) -> Option<&str> {
        self.query
            .default_scope
            .as_deref()
            .or(self.default_scope.as_deref())
    }

    pub fn query_default_limit(&self) -> Option<usize> {
        self.query.default_limit.or(self.default_limit)
    }

    pub fn workflow_default_scope(&self) -> Option<&str> {
        self.workflow
            .default_scope
            .as_deref()
            .or(self.default_scope.as_deref())
    }

    pub fn workflow_default_limit(&self) -> Option<usize> {
        self.workflow.default_limit.or(self.default_limit)
    }
}

fn read_config(path: &Path) -> Result<Config> {
    if !path.exists() {
        return Ok(Config::default());
    }
    let content =
        fs::read_to_string(path).with_context(|| format!("read config {}", path.display()))?;
    toml::from_str(&content).with_context(|| format!("parse config {}", path.display()))
}

pub fn user_config_path() -> PathBuf {
    env::var("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| home_dir().join(".config"))
        .join("agent-knowledge")
        .join("config.toml")
}

pub fn expand_home(path: &str) -> PathBuf {
    if path == "~" {
        return home_dir();
    }
    if let Some(rest) = path.strip_prefix("~/") {
        return home_dir().join(rest);
    }
    PathBuf::from(path)
}

fn home_dir() -> PathBuf {
    env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overlay_prefers_higher_priority_values() {
        let store = Config {
            default_scope: Some("global".to_string()),
            default_limit: Some(20),
            query: QueryConfig {
                default_scope: Some("project:store/repo".to_string()),
                default_limit: Some(10),
            },
            ..Config::default()
        };
        let user = Config {
            default_limit: Some(50),
            query: QueryConfig {
                default_scope: Some("auto".to_string()),
                ..QueryConfig::default()
            },
            ..Config::default()
        };

        let merged = store.overlay(&user);

        assert_eq!(merged.query_default_scope(), Some("auto"));
        assert_eq!(merged.query_default_limit(), Some(50));
        assert_eq!(merged.workflow_default_scope(), Some("global"));
        assert_eq!(merged.workflow_default_limit(), Some(50));
    }
}
