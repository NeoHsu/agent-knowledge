use std::env;
use std::fs::{self, File};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use fs2::FileExt;
use rusqlite::Connection;

use crate::{db::migrate_schema, search_index};

#[derive(Debug)]
pub struct App {
    pub root: PathBuf,
    pub db_path: PathBuf,
    pub index_path: PathBuf,
    pub schema_path: PathBuf,
}

impl App {
    pub fn discover() -> Result<Self> {
        let exe_dir = env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(Path::to_path_buf));
        let cwd = env::current_dir().context("read current directory")?;
        let root = if cwd.join("schema/memory-schema.sql").exists() {
            cwd
        } else if let Some(dir) = exe_dir {
            find_root(&dir).unwrap_or_else(default_root)
        } else {
            default_root()
        };

        Ok(Self {
            db_path: root.join("memory.db"),
            index_path: root.join("index"),
            schema_path: root.join("schema/memory-schema.sql"),
            root,
        })
    }

    pub fn init(&self) -> Result<()> {
        fs::create_dir_all(&self.index_path).context("create index directory")?;
        let conn = self.conn()?;
        let schema =
            fs::read_to_string(&self.schema_path).context("read schema/memory-schema.sql")?;
        conn.execute_batch(&schema)
            .context("apply database schema")?;
        migrate_schema(&conn)?;
        search_index::ensure(&self.index_path)?;
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

fn default_root() -> PathBuf {
    env::var("AGENT_KNOWLEDGE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| home_dir().join(".agent-knowledge"))
}

fn home_dir() -> PathBuf {
    env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."))
}
