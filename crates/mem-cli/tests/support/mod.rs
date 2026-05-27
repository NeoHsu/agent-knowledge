#![allow(dead_code)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{params, Connection};

pub struct TestRepo {
    path: PathBuf,
}

impl TestRepo {
    pub fn new(name: &str) -> Self {
        Self {
            path: temp_repo(name),
        }
    }

    #[allow(dead_code)]
    pub fn path(&self) -> &PathBuf {
        &self.path
    }

    pub fn join(&self, path: impl AsRef<Path>) -> PathBuf {
        self.path.join(path)
    }

    pub fn run(&self, args: &[&str]) -> String {
        run(&self.path, args)
    }

    #[allow(dead_code)]
    pub fn run_fail(&self, args: &[&str]) -> String {
        run_fail(&self.path, args)
    }

    pub fn insert_raw_memory(&self, id: &str, name: &str, content: &str) {
        insert_raw_memory(&self.path, id, name, content);
    }
}

impl Drop for TestRepo {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.path).ok();
    }
}

pub fn mem_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_mem"))
}

pub fn temp_repo(name: &str) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("agent-knowledge-{name}-{stamp}"));
    fs::create_dir_all(dir.join("schema")).expect("schema dir");
    fs::write(
        dir.join("schema/memory-schema.sql"),
        include_str!("../../../../schema/memory-schema.sql"),
    )
    .expect("schema");
    dir
}

pub fn run(repo: &PathBuf, args: &[&str]) -> String {
    let output = Command::new(mem_bin())
        .current_dir(repo)
        .args(args)
        .output()
        .expect("run mem");
    assert!(
        output.status.success(),
        "command failed: {:?}\nstdout={}\nstderr={}",
        args,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("utf8 stdout")
}

#[allow(dead_code)]
pub fn run_fail(repo: &PathBuf, args: &[&str]) -> String {
    let output = Command::new(mem_bin())
        .current_dir(repo)
        .args(args)
        .output()
        .expect("run mem");
    assert!(
        !output.status.success(),
        "command unexpectedly succeeded: {:?}\nstdout={}\nstderr={}",
        args,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

pub fn insert_raw_memory(repo: &Path, id: &str, name: &str, content: &str) {
    let conn = Connection::open(repo.join("memory.db")).expect("open memory db");
    conn.execute(
        "INSERT INTO memories
        (id, type, name, content, tags, scope, source, confidence, protected, created_at, updated_at)
        VALUES (?1, 'reference', ?2, ?3, '[]', 'global', 'manual', 'high', 1, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
        params![id, name, content],
    )
    .expect("insert raw memory");
}
