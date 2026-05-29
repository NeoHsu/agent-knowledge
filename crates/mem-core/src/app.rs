use std::env;
use std::fs::{self, File};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use fs2::FileExt;
use rusqlite::Connection;

use crate::config::{expand_home, Config};
use crate::{db::migrate_schema, search_index};

const MEMORY_SCHEMA: &str = include_str!("../../../schema/memory-schema.sql");

#[derive(Debug)]
pub struct App {
    pub root: PathBuf,
    pub db_path: PathBuf,
    pub index_path: PathBuf,
    pub config: Config,
}

impl App {
    pub fn discover() -> Result<Self> {
        let user_config = Config::load_user()?;
        let exe_dir = env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(Path::to_path_buf));
        let cwd = env::current_dir().context("read current directory")?;
        let root = if cwd.join("schema/memory-schema.sql").exists() {
            cwd
        } else if let Some(dir) = exe_dir {
            find_root(&dir).unwrap_or_else(|| default_root(&user_config))
        } else {
            default_root(&user_config)
        };
        let config = Config::merged_for_root(&root, &user_config)?;

        Ok(Self {
            db_path: root.join("memory.db"),
            index_path: root.join("index"),
            root,
            config,
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

fn default_root(config: &Config) -> PathBuf {
    env::var("AGENT_KNOWLEDGE_HOME")
        .map(|path| expand_home(&path))
        .unwrap_or_else(|_| {
            config
                .knowledge_home_path()
                .unwrap_or_else(|| expand_home("~/.agent-knowledge"))
        })
}
