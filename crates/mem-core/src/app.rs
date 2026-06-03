use std::env;
use std::fs::{self, File};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use fs2::FileExt;
use rusqlite::Connection;

use crate::config::{expand_home, Config};
use crate::{db::migrate_schema, search_index};

const MEMORY_SCHEMA: &str = include_str!("../../../schema/memory-schema.sql");

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoreSource {
    CliOverride,
    CurrentDirectory,
    ExecutableParent,
    Environment,
    UserConfig,
    Default,
}

impl StoreSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::CliOverride => "cli",
            Self::CurrentDirectory => "current_directory",
            Self::ExecutableParent => "executable_parent",
            Self::Environment => "environment",
            Self::UserConfig => "user_config",
            Self::Default => "default",
        }
    }
}

#[derive(Debug)]
pub struct App {
    pub root: PathBuf,
    pub db_path: PathBuf,
    pub index_path: PathBuf,
    pub config: Config,
    pub store_source: StoreSource,
}

impl App {
    pub fn discover() -> Result<Self> {
        Self::discover_with_home(None)
    }

    pub fn discover_with_home(home: Option<&str>) -> Result<Self> {
        let user_config = Config::load_user()?;
        let exe_dir = env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(Path::to_path_buf));
        let cwd = env::current_dir().context("read current directory")?;
        let (root, store_source) = if let Some(home) = home {
            (expand_home(home), StoreSource::CliOverride)
        } else if cwd.join("schema/memory-schema.sql").exists() {
            (cwd, StoreSource::CurrentDirectory)
        } else if let Some(dir) = exe_dir {
            find_root(&dir)
                .map(|root| (root, StoreSource::ExecutableParent))
                .unwrap_or_else(|| default_root(&user_config))
        } else {
            default_root(&user_config)
        };
        let config = Config::merged_for_root(&root, &user_config)?;

        Ok(Self {
            db_path: root.join("memory.db"),
            index_path: root.join("index"),
            root,
            config,
            store_source,
        })
    }

    pub fn init(&self) -> Result<()> {
        self.ensure_schema()?;
        fs::create_dir_all(&self.index_path).context("create index directory")?;
        search_index::ensure(&self.index_path)?;
        Ok(())
    }

    pub fn ensure_schema(&self) -> Result<()> {
        fs::create_dir_all(&self.root).context("create app root")?;
        let conn = self.conn()?;
        conn.execute_batch(MEMORY_SCHEMA)
            .context("apply database schema")?;
        migrate_schema(&conn)?;
        Ok(())
    }

    pub fn conn(&self) -> Result<Connection> {
        let conn = Connection::open(&self.db_path)
            .with_context(|| format!("open {}", self.db_path.display()))?;
        conn.execute_batch("PRAGMA foreign_keys = ON; PRAGMA journal_mode = WAL;")?;
        Ok(conn)
    }
}

pub fn with_lock<F>(app: &App, f: F) -> Result<()>
where
    F: FnOnce() -> Result<()>,
{
    fs::create_dir_all(&app.root)?;
    let lock_path = app.root.join(".mem.lock");
    let lock = File::create(lock_path)?;
    lock.lock_exclusive()?;
    let result = f();
    FileExt::unlock(&lock)?;
    result
}

fn find_root(start: &Path) -> Option<PathBuf> {
    let mut current = Some(start);
    while let Some(path) = current {
        if path.join("schema/memory-schema.sql").exists() {
            return Some(path.to_path_buf());
        }
        current = path.parent();
    }
    None
}

fn default_root(config: &Config) -> (PathBuf, StoreSource) {
    if let Ok(path) = env::var("MNEMARK_HOME") {
        return (expand_home(&path), StoreSource::Environment);
    }
    if let Some(path) = config.knowledge_home_path() {
        return (path, StoreSource::UserConfig);
    }
    (expand_home("~/.mnemark"), StoreSource::Default)
}
