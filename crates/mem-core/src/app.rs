use std::env;
use std::fs::{self, OpenOptions};
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result};
use chrono::Utc;
use fs2::FileExt;
use rusqlite::{Connection, OpenFlags};
use uuid::Uuid;

use crate::config::{Config, expand_home};
use crate::db::{
    ensure_store_id, migrate_schema, schema_compatibility_required, supported_schema_version,
    validate_store_schema_objects,
};
use crate::error;
use crate::search_index;

const MEMORY_SCHEMA: &str = include_str!("../../../schema/memory-schema.sql");

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoreSource {
    CliOverride,
    Environment,
    UserConfig,
    Default,
}

impl StoreSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::CliOverride => "cli",
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
        Self::discover_runtime_with_home(home)
    }

    /// Discover the runtime store only: `--home`, `MNEMARK_HOME`, user config,
    /// then `~/.mnemark`. Skips current-directory and executable-adjacent
    /// schema detection so runtime-facing commands (prime, doctor, sync)
    /// never mistake a source checkout for the active store.
    pub fn discover_runtime_with_home(home: Option<&str>) -> Result<Self> {
        let user_config = Config::load_user()?;
        let (root, store_source) = if let Some(home) = home {
            (expand_home(home), StoreSource::CliOverride)
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
        let existing_store = validate_database_path(&self.db_path)?;
        if existing_store {
            self.require_schema()?;
        } else {
            fs::create_dir_all(&self.root).context("create app root")?;
            let conn = Connection::open(&self.db_path)
                .with_context(|| format!("create {}", self.db_path.display()))?;
            conn.execute_batch("PRAGMA foreign_keys = ON; PRAGMA journal_mode = WAL;")?;
            conn.execute_batch(MEMORY_SCHEMA)
                .context("apply database schema")?;
            conn.pragma_update(None, "user_version", supported_schema_version())?;
            ensure_store_id(&conn)?;
            conn.execute(
                "INSERT INTO metadata (key, value, updated_at)
                 VALUES (?1, ?2, CURRENT_TIMESTAMP)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
                rusqlite::params![
                    crate::graph::GRAPH_SCHEMA_VERSION_KEY,
                    crate::graph::GRAPH_SCHEMA_VERSION.to_string()
                ],
            )?;
            crate::graph::set_graph_dirty(&conn, true)?;
        }
        if existing_store {
            if search_index::validate_existing(&self.index_path).is_err() {
                crate::index::reindex(self).context("rebuild index for existing store")?;
            }
        } else {
            fs::create_dir_all(&self.index_path).context("create index directory")?;
            search_index::ensure(&self.index_path)?;
        }
        harden_store_permissions(&self.root, &self.db_path)?;
        Ok(())
    }

    pub fn schema_version(&self) -> Result<i64> {
        let conn = self.read_conn()?;
        conn.query_row("PRAGMA user_version", [], |row| row.get(0))
            .context("read database schema version")
    }

    pub fn require_schema(&self) -> Result<()> {
        if !self.db_path.exists() {
            return Err(error::not_found(format!(
                "memory store not found at {}; run `mem init` explicitly",
                self.db_path.display()
            )));
        }
        let actual = self.schema_version()?;
        let supported = supported_schema_version();
        if actual < supported {
            return Err(error::compatibility(format!(
                "database schema v{actual} requires explicit migration to v{supported}; \
                 back up the store and run `mem migrate --dry-run`, then `mem migrate`"
            )));
        }
        if actual > supported {
            return Err(error::compatibility(format!(
                "database schema v{actual} is newer than this binary supports (v{supported})"
            )));
        }
        let conn = self.read_conn()?;
        let has_store_id: bool = conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM metadata WHERE key = 'store_id' AND value <> '')",
                [],
                |row| row.get(0),
            )
            .unwrap_or(false);
        if !has_store_id {
            return Err(error::compatibility(
                "store identity metadata is missing; run `mem migrate` explicitly to repair it",
            ));
        }
        if schema_compatibility_required(&conn)? {
            return Err(error::compatibility(format!(
                "database schema v{actual} requires explicit compatibility repair; \
                 run `mem migrate --dry-run`, then `mem migrate`"
            )));
        }
        validate_store_schema_objects(&conn)
            .context("store contains unexpected schema objects; restore a trusted backup")?;
        Ok(())
    }

    pub fn migrate(&self) -> Result<Option<PathBuf>> {
        if !self.db_path.exists() {
            return Err(error::not_found(format!(
                "memory store not found at {}; run `mem init` explicitly",
                self.db_path.display()
            )));
        }
        let current = self.schema_version()?;
        let supported = supported_schema_version();
        if current > supported {
            return Err(error::compatibility(format!(
                "database schema v{current} is newer than this binary supports (v{supported})"
            )));
        }
        if current == supported {
            let conn = self.read_conn()?;
            if !schema_compatibility_required(&conn)? {
                validate_store_schema_objects(&conn).context(
                    "store contains unexpected schema objects; restore a trusted backup",
                )?;
                drop(conn);
                harden_store_permissions(&self.root, &self.db_path)?;
                return Ok(None);
            }
        }

        let stamp = Utc::now().format("%Y%m%dT%H%M%SZ");
        let backup = self.root.join(format!(
            "memory.db.backup-{stamp}-{}",
            Uuid::new_v4().simple()
        ));
        let backup_result = (|| -> Result<String> {
            let source = self.read_conn()?;
            let mut destination = Connection::open(&backup)
                .with_context(|| format!("create migration backup {}", backup.display()))?;
            rusqlite::backup::Backup::new(&source, &mut destination)?
                .run_to_completion(128, Duration::from_millis(10), None)
                .context("create consistent SQLite migration backup")?;
            destination
                .query_row("PRAGMA quick_check", [], |row| row.get(0))
                .map_err(Into::into)
        })();
        let backup_check = match backup_result {
            Ok(check) => check,
            Err(error) => {
                fs::remove_file(&backup).ok();
                return Err(error);
            }
        };
        if backup_check != "ok" {
            fs::remove_file(&backup).ok();
            return Err(error::integrity(format!(
                "migration backup failed SQLite quick_check: {backup_check}"
            )));
        }
        harden_file_permissions(&backup)?;

        let conn = self.conn_unchecked()?;
        conn.execute_batch("BEGIN IMMEDIATE TRANSACTION;")?;
        if let Err(error) = migrate_schema(&conn) {
            let _ = conn.execute_batch("ROLLBACK;");
            return Err(error).context("migrate database; original backup preserved");
        }
        let validation = (|| -> Result<()> {
            if schema_compatibility_required(&conn)? {
                return Err(error::compatibility(
                    "compatibility invariants are still missing after migration",
                ));
            }
            validate_store_schema_objects(&conn)?;
            Ok(())
        })();
        if let Err(error) = validation {
            let _ = conn.execute_batch("ROLLBACK;");
            return Err(error).context("validate migrated schema; original backup preserved");
        }
        if let Err(error) = conn.execute_batch("COMMIT;") {
            let _ = conn.execute_batch("ROLLBACK;");
            return Err(error).context("commit database migration");
        }
        let quick_check: String = conn.query_row("PRAGMA quick_check", [], |row| row.get(0))?;
        if quick_check != "ok" {
            return Err(error::integrity(format!(
                "database migration completed but quick_check returned {quick_check:?}; \
                 stop using the store and restore {}",
                backup.display()
            )));
        }
        if schema_compatibility_required(&conn)? {
            return Err(error::compatibility(format!(
                "database migration completed but compatibility invariants are still missing; \
                 stop using the store and restore {}",
                backup.display()
            )));
        }
        validate_store_schema_objects(&conn).with_context(|| {
            format!(
                "database migration completed with unexpected schema objects; restore {}",
                backup.display()
            )
        })?;
        harden_store_permissions(&self.root, &self.db_path)?;
        Ok(Some(backup))
    }

    pub fn read_conn(&self) -> Result<Connection> {
        if !validate_database_path(&self.db_path)? {
            return Err(error::not_found(format!(
                "memory store not found at {}; run `mem init` explicitly",
                self.db_path.display()
            )));
        }
        let conn = Connection::open_with_flags(
            &self.db_path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI,
        )
        .with_context(|| format!("open {} read-only", self.db_path.display()))?;
        conn.execute_batch("PRAGMA foreign_keys = ON;")?;
        Ok(conn)
    }

    pub fn conn(&self) -> Result<Connection> {
        self.require_schema()?;
        self.conn_unchecked()
    }

    pub fn harden_permissions(&self) -> Result<()> {
        harden_store_permissions(&self.root, &self.db_path)
    }

    fn conn_unchecked(&self) -> Result<Connection> {
        validate_database_path(&self.db_path)?;
        let conn = Connection::open(&self.db_path)
            .with_context(|| format!("open {}", self.db_path.display()))?;
        conn.execute_batch("PRAGMA foreign_keys = ON; PRAGMA journal_mode = WAL;")?;
        Ok(conn)
    }
}

pub fn with_shared_lock<F>(app: &App, f: F) -> Result<()>
where
    F: FnOnce() -> Result<()>,
{
    fs::create_dir_all(&app.root)?;
    harden_directory_permissions(&app.root)?;
    let lock_path = app.root.join(".mem.lock");
    if let Ok(metadata) = fs::symlink_metadata(&lock_path) {
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(error::safety_violation(format!(
                "refusing unsafe lock path: {}",
                lock_path.display()
            )));
        }
    }
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&lock_path)?;
    harden_file_permissions(&lock_path)?;
    FileExt::lock_shared(&lock)?;
    let result = f();
    FileExt::unlock(&lock)?;
    result
}

pub fn with_lock<F>(app: &App, f: F) -> Result<()>
where
    F: FnOnce() -> Result<()>,
{
    fs::create_dir_all(&app.root)?;
    harden_directory_permissions(&app.root)?;
    let lock_path = app.root.join(".mem.lock");
    if let Ok(metadata) = fs::symlink_metadata(&lock_path) {
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(error::safety_violation(format!(
                "refusing unsafe lock path: {}",
                lock_path.display()
            )));
        }
    }
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&lock_path)?;
    harden_file_permissions(&lock_path)?;
    lock.lock_exclusive()?;
    if app.db_path.exists() {
        harden_file_permissions(&app.db_path)?;
    }
    let result = f();
    FileExt::unlock(&lock)?;
    result
}

fn validate_database_path(path: &std::path::Path) -> Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => Err(
            error::safety_violation(format!("refusing unsafe database path: {}", path.display())),
        ),
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
}

fn harden_store_permissions(root: &std::path::Path, db: &std::path::Path) -> Result<()> {
    harden_directory_permissions(root)?;
    harden_file_permissions(db)
}

#[cfg(unix)]
fn harden_directory_permissions(path: &std::path::Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
        .with_context(|| format!("set permissions on {}", path.display()))
}

#[cfg(not(unix))]
fn harden_directory_permissions(_path: &std::path::Path) -> Result<()> {
    Ok(())
}

#[cfg(unix)]
fn harden_file_permissions(path: &std::path::Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
        .with_context(|| format!("set permissions on {}", path.display()))
}

#[cfg(not(unix))]
fn harden_file_permissions(_path: &std::path::Path) -> Result<()> {
    Ok(())
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
